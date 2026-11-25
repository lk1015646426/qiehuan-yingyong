//! Shared bounded local request dispatch and per-account admission.
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use tiny_http::{Request, Response, Server, StatusCode};

pub(crate) const MAX_ACTIVE: usize = 8;

// Waiting for a contended account slot is bounded: an overloaded pool still
// surfaces local_accounts_busy to the client instead of queueing without end.
pub(crate) const SLOT_WAIT_MAX: Duration = Duration::from_secs(120);

#[derive(Clone, Default)]
pub(crate) struct AccountSlots(Arc<SlotState>);

#[derive(Default)]
struct SlotState {
    busy: Mutex<HashSet<String>>,
    free: Condvar,
}

pub(crate) struct AccountLease {
    slots: AccountSlots,
    id: String,
}

impl AccountSlots {
    // A single in-flight request per account until provider limits are verified.
    pub(crate) fn try_acquire(&self, id: &str) -> Option<AccountLease> {
        let mut busy = self.0.busy.lock().unwrap_or_else(|e| e.into_inner());
        if !busy.insert(id.to_owned()) {
            return None;
        }
        Some(AccountLease {
            slots: self.clone(),
            id: id.to_owned(),
        })
    }

    /// Bounded wait until any slot is released. Returns false on timeout or
    /// shutdown; callers then rescan candidates with try_acquire. Shutdown is
    /// noticed within 250ms without relying on a notify.
    pub(crate) fn wait_any_release(&self, stopping: &AtomicBool, max_wait: Duration) -> bool {
        let deadline = Instant::now() + max_wait;
        let mut busy = self.0.busy.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if stopping.load(Ordering::SeqCst) {
                return false;
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            let slice = deadline.min(now + Duration::from_millis(250));
            let (guard, result) = self
                .0
                .free
                .wait_timeout(busy, slice - now)
                .unwrap_or_else(|e| e.into_inner());
            busy = guard;
            if !result.timed_out() {
                return true;
            }
        }
    }
}

impl Drop for AccountLease {
    fn drop(&mut self) {
        self.slots
            .0
            .busy
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.id);
        self.slots.0.free.notify_all();
    }
}

fn reject(request: Request, status: u16, message: &str) {
    let reason = if status == 429 {
        "local_pool_busy"
    } else {
        "local_pool_stopping"
    };
    let body =
        serde_json::json!({"error":{"message":message,"type":"local_pool_error","code":reason}});
    let mut response = Response::from_string(body.to_string())
        .with_status_code(StatusCode(status))
        .with_header(tiny_http::Header::from_bytes("Content-Type", "application/json").unwrap());
    if status == 429 {
        response.add_header(tiny_http::Header::from_bytes("Retry-After", "15").unwrap());
    }
    let _ = request.respond(response);
}

// The listener never waits on upstream I/O. Saturated requests receive backpressure;
// shutdown joins already admitted workers before releasing the listener.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn dispatch(server: &Server, stopping: &AtomicBool, handler: impl Fn(Request) + Sync) {
    dispatch_observed(server, stopping, handler, |_| {});
}

// Only report local admission outcomes; do not alter scheduling or provider limits.
pub(crate) fn dispatch_observed(
    server: &Server,
    stopping: &AtomicBool,
    handler: impl Fn(Request) + Sync,
    observe_rejection: impl Fn(u16),
) {
    std::thread::scope(|scope| {
        let mut workers: Vec<std::thread::ScopedJoinHandle<'_, ()>> = Vec::new();
        while !stopping.load(Ordering::SeqCst) {
            let mut i = 0;
            while i < workers.len() {
                if workers[i].is_finished() {
                    let _ = workers.swap_remove(i).join();
                } else {
                    i += 1;
                }
            }
            match server.recv_timeout(Duration::from_millis(100)) {
                Ok(Some(request)) => {
                    if stopping.load(Ordering::SeqCst) {
                        observe_rejection(503);
                        reject(request, 503, "Local pool stopping");
                        break;
                    }
                    if workers.len() >= MAX_ACTIVE {
                        observe_rejection(429);
                        reject(request, 429, "Local pool busy; retry later");
                        continue;
                    }
                    let handle = &handler;
                    match std::thread::Builder::new()
                        .name("model-pool-request".into())
                        .spawn_scoped(scope, move || handle(request))
                    {
                        Ok(worker) => workers.push(worker),
                        Err(_) => { /* request is dropped; no upstream replay */ }
                    }
                }
                Ok(None) => {}
                Err(_) => break,
            }
        }
        for worker in workers {
            let _ = worker.join();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::sync::{mpsc, Condvar};

    #[test]
    fn account_lease_is_exclusive_and_released_on_drop() {
        let slots = AccountSlots::default();
        let a = slots.try_acquire("a").unwrap();
        assert!(slots.try_acquire("a").is_none());
        assert!(slots.try_acquire("b").is_some());
        drop(a);
        assert!(slots.try_acquire("a").is_some());
    }

    #[test]
    fn wait_any_release_wakes_when_a_slot_is_released() {
        let slots = AccountSlots::default();
        let held = slots.try_acquire("a").unwrap();
        let stopping = AtomicBool::new(false);
        let started = Instant::now();
        let acquired = std::thread::scope(|scope| {
            let waiter = scope.spawn(|| slots.wait_any_release(&stopping, Duration::from_secs(5)));
            std::thread::sleep(Duration::from_millis(150));
            drop(held);
            waiter.join().unwrap()
        });
        assert!(acquired, "a release must wake the waiter");
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "wakeup must not wait for the deadline"
        );
        assert!(
            slots.try_acquire("a").is_some(),
            "the released slot must be acquirable again"
        );
    }

    #[test]
    fn wait_any_release_times_out_when_nothing_is_released() {
        let slots = AccountSlots::default();
        let _held = slots.try_acquire("a").unwrap();
        let stopping = AtomicBool::new(false);
        let started = Instant::now();
        let woken = std::thread::scope(|scope| {
            scope
                .spawn(|| slots.wait_any_release(&stopping, Duration::from_millis(300)))
                .join()
                .unwrap()
        });
        assert!(!woken);
        assert!(started.elapsed() >= Duration::from_millis(300));
    }

    #[test]
    fn wait_any_release_stops_promptly_on_shutdown() {
        let slots = AccountSlots::default();
        let _held = slots.try_acquire("a").unwrap();
        let stopping = AtomicBool::new(true);
        let started = Instant::now();
        let woken = std::thread::scope(|scope| {
            scope
                .spawn(|| slots.wait_any_release(&stopping, Duration::from_secs(30)))
                .join()
                .unwrap()
        });
        assert!(!woken);
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "shutdown must not wait for the deadline"
        );
    }

    #[test]
    fn rejected_request_is_json_and_never_reaches_the_handler() {
        use std::sync::atomic::AtomicUsize;
        let server = Arc::new(Server::http("127.0.0.1:0").unwrap());
        let endpoint = format!("http://{}", server.server_addr());
        let stopping = Arc::new(AtomicBool::new(false));
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let served = Arc::new(AtomicUsize::new(0));
        let (tx, rx) = mpsc::channel();
        let (s, stop, g, count) = (
            server.clone(),
            stopping.clone(),
            gate.clone(),
            served.clone(),
        );
        let listener = std::thread::spawn(move || {
            dispatch(&s, &stop, |mut req| {
                let mut body = String::new();
                let _ = req.as_reader().read_to_string(&mut body);
                count.fetch_add(1, Ordering::SeqCst);
                tx.send(()).unwrap();
                let (lock, cv) = &*g;
                let guard = lock.lock().unwrap();
                let _guard = cv.wait_while(guard, |ready| !*ready).unwrap();
                let _ = req.respond(Response::from_string(body));
            })
        });
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(15))
            .no_proxy()
            .build()
            .unwrap();
        let mut callers = Vec::new();
        for i in 0..MAX_ACTIVE {
            let (client, url) = (client.clone(), endpoint.clone());
            callers.push(std::thread::spawn(move || {
                let body = format!("held-{i}");
                let response = client.post(url).body(body.clone()).send().unwrap();
                assert_eq!(response.status(), 200);
                assert_eq!(response.text().unwrap(), body);
            }));
        }
        let all_started = (0..MAX_ACTIVE).all(|_| rx.recv_timeout(Duration::from_secs(5)).is_ok());
        let rejected = if all_started {
            client
                .post(&endpoint)
                .header("Authorization", "Bearer not-a-real-key")
                .body("must-not-be-forwarded")
                .send()
                .ok()
        } else {
            None
        };
        let status = rejected.as_ref().map(|r| r.status().as_u16());
        let content_type = rejected
            .as_ref()
            .and_then(|r| r.headers().get("Content-Type"))
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let retry_after = rejected
            .as_ref()
            .and_then(|r| r.headers().get("Retry-After"))
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let body = rejected
            .map(|r| r.text().unwrap_or_default())
            .unwrap_or_default();
        assert_eq!(
            served.load(Ordering::SeqCst),
            MAX_ACTIVE,
            "overflow request must not reach the handler"
        );
        *gate.0.lock().unwrap() = true;
        gate.1.notify_all();
        stopping.store(true, Ordering::SeqCst);
        server.unblock();
        for caller in callers {
            caller.join().unwrap();
        }
        listener.join().unwrap();
        assert!(all_started);
        assert_eq!(status, Some(429));
        assert_eq!(content_type.as_deref(), Some("application/json"));
        let parsed: serde_json::Value =
            serde_json::from_str(&body).expect("rejection must be JSON");
        assert_eq!(parsed["error"]["type"], "local_pool_error");
        assert_eq!(parsed["error"]["code"], "local_pool_busy");
        assert_eq!(retry_after.as_deref(), Some("15"));
        assert!(!body.contains("must-not-be-forwarded"));
    }

    #[test]
    fn eight_parallel_requests_are_isolated_and_ninth_is_rejected() {
        let server = Arc::new(Server::http("127.0.0.1:0").unwrap());
        let endpoint = format!("http://{}", server.server_addr());
        let stopping = Arc::new(AtomicBool::new(false));
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let (tx, rx) = mpsc::channel();
        let (s, stop, g) = (server.clone(), stopping.clone(), gate.clone());
        let (reject_tx, reject_rx) = mpsc::channel();
        let listener = std::thread::spawn(move || {
            dispatch_observed(
                &s,
                &stop,
                |mut req| {
                    let mut body = String::new();
                    req.as_reader().read_to_string(&mut body).unwrap();
                    tx.send(()).unwrap();
                    let (lock, cv) = &*g;
                    let guard = lock.lock().unwrap();
                    let _guard = cv.wait_while(guard, |ready| !*ready).unwrap();
                    let _ = req.respond(Response::from_string(body));
                },
                |status| {
                    let _ = reject_tx.send(status);
                },
            )
        });
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(15))
            .no_proxy()
            .build()
            .unwrap();
        let mut callers = Vec::new();
        for i in 0..MAX_ACTIVE {
            let (client, url) = (client.clone(), endpoint.clone());
            callers.push(std::thread::spawn(move || {
                let history = format!("user-{i}: first; assistant-{i}: reply; user-{i}: follow-up");
                let response = client.post(url).body(history.clone()).send().unwrap();
                assert_eq!(response.status(), 200);
                assert_eq!(response.text().unwrap(), history);
            }));
        }
        let all_started = (0..MAX_ACTIVE).all(|_| rx.recv_timeout(Duration::from_secs(5)).is_ok());
        let ninth = if all_started {
            client
                .post(&endpoint)
                .body("extra")
                .send()
                .ok()
                .map(|r| r.status().as_u16())
        } else {
            None
        };
        *gate.0.lock().unwrap() = true;
        gate.1.notify_all();
        stopping.store(true, Ordering::SeqCst);
        server.unblock();
        for caller in callers {
            caller.join().unwrap();
        }
        listener.join().unwrap();
        assert!(all_started);
        assert_eq!(ninth, Some(429));
        assert_eq!(reject_rx.try_iter().collect::<Vec<_>>(), vec![429]);
    }
}

/// Opt-in admission. Stores only ticket metadata; no queued request bodies or replay.
/// Grants bounded concurrent leases instead of serializing: ACTIVE_CAP leases across
/// all clients and PER_CLIENT_CAP per client. The front-of-order window keeps claiming
/// round-robin fair between clients; the listener still caps total workers at
/// MAX_ACTIVE and each account at one in-flight request.
mod admission {
    use std::collections::{HashMap, VecDeque};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    /// Concurrent leases handed out across all clients.
    const ACTIVE_CAP: usize = 4;
    /// Concurrent leases a single client may hold, so one client cannot monopolize the pool.
    const PER_CLIENT_CAP: usize = 3;
    #[derive(Clone)]
    pub(crate) struct Queue(Arc<Mutex<State>>);
    impl Default for Queue {
        fn default() -> Self {
            Queue(Arc::new(Mutex::new(State::default())))
        }
    }
    struct State {
        order: VecDeque<String>,
        clients: HashMap<String, VecDeque<Ticket>>,
        active: HashMap<String, usize>,
        active_total: usize,
        pending: usize,
        active_cap: usize,
        per_client_cap: usize,
    }
    impl Default for State {
        fn default() -> Self {
            Self {
                order: VecDeque::new(),
                clients: HashMap::new(),
                active: HashMap::new(),
                active_total: 0,
                pending: 0,
                active_cap: ACTIVE_CAP,
                per_client_cap: PER_CLIENT_CAP,
            }
        }
    }
    struct Ticket {
        id: String,
        expires: Instant,
        idle_until: Instant,
    }
    pub(crate) struct Lease {
        queue: Queue,
        client: String,
        id: String,
    }
    impl Drop for Lease {
        fn drop(&mut self) {
            let mut s = self.queue.0.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(n) = s.active.get_mut(&self.client) {
                *n = n.saturating_sub(1);
                if *n == 0 {
                    s.active.remove(&self.client);
                }
            }
            s.active_total = s.active_total.saturating_sub(1);
        }
    }
    impl State {
        fn expire(&mut self, now: Instant) {
            let mut n = 0;
            self.clients.retain(|_, q| {
                q.retain(|t| t.expires > now && t.idle_until > now);
                n += q.len();
                !q.is_empty()
            });
            self.order.retain(|c| self.clients.contains_key(c));
            self.pending = n;
        }
        fn eligible(&self, client: &str, id: &str) -> bool {
            self.active_total < self.active_cap
                && self
                    .active
                    .get(client)
                    .map_or(true, |n| *n < self.per_client_cap)
                && self.order.iter().take(self.active_cap).any(|c| c == client)
                && self
                    .clients
                    .get(client)
                    // Front-of-queue only: reporting every ticket ready would let a
                    // freed lease trigger a claim race that 409s the losers.
                    .is_some_and(|q| q.front().is_some_and(|t| t.id == id))
        }
    }
    impl Queue {
        /// Pool-specific lease bounds; expiry and round-robin semantics unchanged.
        pub(crate) fn with_caps(active_cap: usize, per_client_cap: usize) -> Self {
            let queue = Queue(Arc::new(Mutex::new(State::default())));
            let mut s = queue.0.lock().unwrap_or_else(|e| e.into_inner());
            s.active_cap = active_cap.max(1);
            s.per_client_cap = per_client_cap.max(1);
            drop(s);
            queue
        }
        pub(crate) fn enqueue(&self, client: &str) -> Option<String> {
            let now = Instant::now();
            let mut s = self.0.lock().unwrap_or_else(|e| e.into_inner());
            s.expire(now);
            if s.pending >= 64 || s.clients.get(client).is_some_and(|q| q.len() >= 8) {
                return None;
            }
            let id = uuid::Uuid::new_v4().to_string();
            if !s.clients.contains_key(client) {
                s.order.push_back(client.to_owned());
            }
            s.clients
                .entry(client.to_owned())
                .or_default()
                .push_back(Ticket {
                    id: id.clone(),
                    expires: now + Duration::from_secs(300),
                    idle_until: now + Duration::from_secs(15),
                });
            s.pending += 1;
            Some(id)
        }
        pub(crate) fn status(&self, client: &str, id: &str) -> Option<bool> {
            let mut s = self.0.lock().unwrap_or_else(|e| e.into_inner());
            s.expire(Instant::now());
            let t = s.clients.get_mut(client)?.iter_mut().find(|t| t.id == id)?;
            t.idle_until = Instant::now() + Duration::from_secs(15);
            Some(s.eligible(client, id))
        }
        pub(crate) fn cancel(&self, client: &str, id: &str) -> bool {
            let mut s = self.0.lock().unwrap_or_else(|e| e.into_inner());
            s.expire(Instant::now());
            let Some(q) = s.clients.get_mut(client) else {
                return false;
            };
            let Some(i) = q.iter().position(|t| t.id == id) else {
                return false;
            };
            q.remove(i);
            if q.is_empty() {
                s.clients.remove(client);
                s.order.retain(|c| c != client);
            }
            s.pending -= 1;
            true
        }
        pub(crate) fn claim(&self, client: &str, id: &str) -> Option<Lease> {
            let mut s = self.0.lock().unwrap_or_else(|e| e.into_inner());
            s.expire(Instant::now());
            if !s.eligible(client, id) {
                return None;
            }
            let more = {
                let q = s.clients.get_mut(client).unwrap();
                let i = q.iter().position(|t| t.id == id).unwrap();
                q.remove(i);
                !q.is_empty()
            };
            if let Some(i) = s.order.iter().position(|c| c == client) {
                s.order.remove(i);
            }
            if more {
                s.order.push_back(client.to_owned());
            } else {
                s.clients.remove(client);
            }
            s.pending -= 1;
            *s.active.entry(client.to_owned()).or_insert(0) += 1;
            s.active_total += 1;
            Some(Lease {
                queue: self.clone(),
                client: client.to_owned(),
                id: id.to_owned(),
            })
        }
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn admission_round_robin_and_exclusive_lease() {
            let q = Queue::default();
            let ids: Vec<_> = (0..8)
                .map(|i| {
                    let c = format!("p{i}");
                    let a = q.enqueue(&c).unwrap();
                    let b = q.enqueue(&c).unwrap();
                    (c, a, b)
                })
                .collect();
            for round in 0..2 {
                for (c, a, b) in &ids {
                    let id = if round == 0 { a } else { b };
                    assert_eq!(q.status(c, id), Some(true));
                    let lease = q.claim(c, id).unwrap();
                    assert!(q.claim(c, id).is_none());
                    assert!(!q.cancel(c, id));
                    drop(lease);
                }
            }
            assert_eq!(q.0.lock().unwrap().pending, 0);
        }
        #[test]
        fn admission_cancel_expire_and_owner() {
            let q = Queue::default();
            let a = q.enqueue("a").unwrap();
            let b = q.enqueue("b").unwrap();
            assert!(!q.cancel("b", &a));
            assert!(q.cancel("a", &a));
            assert!(q.claim("a", &a).is_none());
            assert_eq!(q.status("b", &b), Some(true));
            q.0.lock().unwrap().clients.get_mut("b").unwrap()[0].expires = Instant::now();
            assert!(q.claim("b", &b).is_none());
            assert_eq!(q.status("b", &b), None);
        }
        #[test]
        fn admission_abandoned_client_does_not_block_next() {
            let q = Queue::default();
            let a = q.enqueue("a").unwrap();
            let b = q.enqueue("b").unwrap();
            q.0.lock().unwrap().clients.get_mut("a").unwrap()[0].idle_until = Instant::now();
            assert_eq!(q.status("a", &a), None);
            assert_eq!(q.status("b", &b), Some(true));
        }
        #[test]
        fn admission_queue_limits_and_panic_release() {
            let q = Queue::default();
            for _ in 0..8 {
                q.enqueue("a").unwrap();
            }
            assert!(q.enqueue("a").is_none());
            for i in 0..56 {
                q.enqueue(&format!("p{i}")).unwrap();
            }
            assert!(q.enqueue("overflow").is_none());
            let id = q.0.lock().unwrap().clients["a"][0].id.clone();
            let _ = std::panic::catch_unwind(|| {
                let _lease = q.claim("a", &id).unwrap();
                panic!("fixture");
            });
            let s = q.0.lock().unwrap();
            assert!(s.active.is_empty());
            assert_eq!(s.active_total, 0);
        }
        #[test]
        fn admission_grants_concurrent_leases_to_one_client() {
            let q = Queue::default();
            let t: Vec<_> = (0..4).map(|_| q.enqueue("a").unwrap()).collect();
            let l1 = q.claim("a", &t[0]).unwrap();
            assert_eq!(q.status("a", &t[1]), Some(true)); // second ticket ready while the first request streams
            let l2 = q.claim("a", &t[1]).unwrap();
            let l3 = q.claim("a", &t[2]).unwrap();
            assert!(q.claim("a", &t[3]).is_none()); // per-client cap reached
            assert_eq!(q.status("a", &t[3]), Some(false));
            drop(l2);
            let l4 = q.claim("a", &t[3]).unwrap(); // released slot is reusable
            drop(l1);
            drop(l3);
            drop(l4);
            let s = q.0.lock().unwrap();
            assert_eq!(s.active_total, 0);
            assert!(s.active.is_empty());
        }
        #[test]
        fn admission_global_cap_spans_clients() {
            let q = Queue::default();
            let a: Vec<_> = (0..3).map(|_| q.enqueue("a").unwrap()).collect();
            let b: Vec<_> = (0..3).map(|_| q.enqueue("b").unwrap()).collect();
            let l1 = q.claim("a", &a[0]).unwrap();
            let l2 = q.claim("b", &b[0]).unwrap();
            let l3 = q.claim("a", &a[1]).unwrap();
            let l4 = q.claim("b", &b[1]).unwrap();
            assert!(q.claim("a", &a[2]).is_none()); // ACTIVE_CAP reached across clients
            assert!(q.claim("b", &b[2]).is_none());
            drop(l3);
            let l5 = q.claim("a", &a[2]).unwrap();
            drop(l1);
            drop(l2);
            drop(l4);
            drop(l5);
            assert_eq!(q.0.lock().unwrap().active_total, 0);
        }
        #[test]
        fn admission_window_keeps_deep_clients_waiting() {
            let q = Queue::default();
            let ids: Vec<String> = (0..6)
                .map(|i| q.enqueue(&format!("c{i}")).unwrap())
                .collect();
            assert_eq!(q.status("c0", &ids[0]), Some(true));
            assert_eq!(q.status("c4", &ids[4]), Some(false)); // beyond the round-robin window
        }
        #[test]
        fn admission_front_ticket_only_is_ready_and_custom_caps_apply() {
            let q = Queue::with_caps(2, 2);
            let a = q.enqueue("a").unwrap();
            let b = q.enqueue("a").unwrap();
            let c = q.enqueue("a").unwrap();
            assert_eq!(q.status("a", &a), Some(true)); // front ticket ready
            assert_eq!(q.status("a", &b), Some(false)); // behind front: no herd
            let l1 = q.claim("a", &a).unwrap();
            assert!(q.claim("a", &c).is_none()); // c is not at the front yet
            assert_eq!(q.status("a", &b), Some(true)); // b promoted to front
            let l2 = q.claim("a", &b).unwrap();
            assert!(q.claim("a", &c).is_none()); // per-client cap reached
            drop(l1);
            let l3 = q.claim("a", &c).unwrap(); // released slot is reusable
            drop(l2);
            drop(l3);
            assert_eq!(q.0.lock().unwrap().active_total, 0);
        }
    }
}
pub(crate) use admission::{Lease as AdmissionLease, Queue as AdmissionQueue};

fn admission_header(request: &Request, name: &str) -> Option<String> {
    request
        .headers()
        .iter()
        .find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(name))
        .map(|h| h.value.as_str().to_owned())
        .filter(|s| {
            s.len() <= 64
                && !s.is_empty()
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        })
}
fn admission_reply(request: Request, status: u16, body: serde_json::Value) {
    let _ = request.respond(
        Response::from_string(body.to_string())
            .with_status_code(StatusCode(status))
            .with_header(tiny_http::Header::from_bytes("Content-Type", "application/json").unwrap())
            .with_header(tiny_http::Header::from_bytes("Cache-Control", "no-store").unwrap()),
    );
}
/// Invoke only after existing authentication has succeeded.
pub(crate) fn admission_route(request: Request, queue: &AdmissionQueue) -> Option<Request> {
    let path = request.url().to_owned();
    if !path.starts_with("/v1/admission/") {
        return Some(request);
    }
    if request.method() != &tiny_http::Method::Post {
        admission_reply(request, 405, serde_json::json!({"error":"method"}));
        return None;
    }
    let Some(client) = admission_header(&request, "X-Admission-Client") else {
        admission_reply(request, 400, serde_json::json!({"error":"client_required"}));
        return None;
    };
    let ticket = admission_header(&request, "X-Admission-Ticket");
    let (status, body) = match path.as_str() {
        "/v1/admission/enqueue" => match queue.enqueue(&client) {
            Some(id) => (
                201,
                serde_json::json!({"ticket":id,"ttlMs":300000,"idleMs":15000}),
            ),
            None => (409, serde_json::json!({"error":"queue_full"})),
        },
        "/v1/admission/status" => {
            match ticket.as_deref().and_then(|id| queue.status(&client, id)) {
                Some(ready) => (200, serde_json::json!({"ready":ready,"pollAfterMs":1000})),
                None => (
                    410,
                    serde_json::json!({"error":"ticket_missing_or_expired"}),
                ),
            }
        }
        "/v1/admission/cancel" => {
            if ticket
                .as_deref()
                .is_some_and(|id| queue.cancel(&client, id))
            {
                (200, serde_json::json!({"cancelled":true}))
            } else {
                (410, serde_json::json!({"error":"ticket_missing_or_active"}))
            }
        }
        _ => (404, serde_json::json!({"error":"route"})),
    };
    admission_reply(request, status, body);
    None
}
pub(crate) fn admission_enter(
    request: Request,
    queue: &AdmissionQueue,
) -> Option<(Request, Option<AdmissionLease>)> {
    if !request
        .headers()
        .iter()
        .any(|h| h.field.equiv("X-Admission-Ticket"))
    {
        return Some((request, None));
    }
    let lease = admission_header(&request, "X-Admission-Client")
        .zip(admission_header(&request, "X-Admission-Ticket"))
        .and_then(|(client, id)| queue.claim(&client, &id));
    if let Some(lease) = lease {
        Some((request, Some(lease)))
    } else {
        admission_reply(
            request,
            409,
            serde_json::json!({"error":{"code":"admission_not_ready","message":"Admission ticket is not eligible"}}),
        );
        None
    }
}
#[cfg(test)]
mod admission_http_tests {
    use super::*;
    #[test]
    fn authenticated_ticket_http_roundtrip_without_upstream() {
        let server = Arc::new(Server::http("127.0.0.1:0").unwrap());
        let endpoint = format!("http://{}", server.server_addr());
        let srv = server.clone();
        let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed = count.clone();
        let worker = std::thread::spawn(move || {
            let stop = AtomicBool::new(false);
            // Fixed number of received requests, no real providers.
            let queue = AdmissionQueue::default();
            for _ in 0..8 {
                let req = srv
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap()
                    .expect("fixture request");
                if !req.headers().iter().any(|h| {
                    h.field.equiv("Authorization") && h.value.as_str() == "Bearer fixture-only"
                }) {
                    admission_reply(req, 401, serde_json::json!({"error":"unauthorized"}));
                    continue;
                }
                if let Some(req) = admission_route(req, &queue) {
                    if let Some((req, _lease)) = admission_enter(req, &queue) {
                        observed.fetch_add(1, Ordering::SeqCst);
                        admission_reply(req, 200, serde_json::json!({"result":"synthetic"}))
                    }
                }
            }
            stop.store(true, Ordering::SeqCst);
        });
        let http = reqwest::blocking::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let req = |path: &str, owner: &str, ticket: Option<&str>, auth: bool| {
            let mut r = http
                .post(format!("{endpoint}{path}"))
                .header("X-Admission-Client", owner);
            if auth {
                r = r.bearer_auth("fixture-only")
            }
            if let Some(id) = ticket {
                r = r.header("X-Admission-Ticket", id)
            }
            r.send().unwrap()
        };
        let en = "/v1/admission/enqueue";
        let st = "/v1/admission/status";
        let ca = "/v1/admission/cancel";
        let run = "/v1/chat/completions";
        assert_eq!(req(en, "a", None, false).status().as_u16(), 401);
        let t = req(en, "a", None, true);
        assert_eq!(t.status().as_u16(), 201);
        let v: serde_json::Value = t.json().unwrap();
        let id = v["ticket"].as_str().unwrap();
        assert_eq!(req(st, "b", Some(id), true).status().as_u16(), 410);
        assert_eq!(
            req(st, "a", Some(id), true)
                .json::<serde_json::Value>()
                .unwrap()["ready"],
            true
        );
        assert_eq!(req(ca, "a", Some(id), true).status().as_u16(), 200);
        assert_eq!(req(run, "a", Some(id), true).status().as_u16(), 409);
        let v = req(en, "b", None, true)
            .json::<serde_json::Value>()
            .unwrap();
        assert_eq!(
            req(run, "b", v["ticket"].as_str(), true).status().as_u16(),
            200
        );
        worker.join().unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }
}
