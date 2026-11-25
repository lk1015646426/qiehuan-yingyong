// 切换应用 - 应用入口
//
// 侧边栏双面板布局（复用上游 pages/sidebar.css 的 .with-sidebar 体系）：
// - TRAE：WorkCnSwitcherPage（账号快照管理 / 一键切换 / 云端签到）
// - WorkBuddy：WorkBuddyPage（账号管理 / 签到 / 积分）
// 页签为前端 state，不引入路由库。

import './App.css';
import { Bot, Cable } from 'lucide-react';
import { HarnessPage } from './pages/HarnessPage';
import { DoubaoPoolPanel } from './components/doubao/DoubaoPoolPanel';
import { useEffect, useState } from 'react';
import traeCnIcon from './assets/icons/trae-cn.png';
import workBuddyIcon from './assets/icons/workbuddy.png';
import { WorkCnSwitcherPage } from './pages/WorkCnSwitcherPage';
import { WorkBuddyPage } from './pages/WorkBuddyPage';
import { useWorkCnStore } from './stores/useWorkCnStore';
import { useWorkBuddyStore } from './stores/useWorkBuddyStore';

type PageKey = 'switcher' | 'workbuddy' | 'doubao' | 'harness';

export default function App() {
  const [page, setPage] = useState<PageKey>('switcher');

  // gh CLI 状态由侧边栏常驻展示，进入应用时探测一次。
  const githubCliStatus = useWorkCnStore((s) => s.githubCliStatus);
  const refreshGitHubCliStatus = useWorkCnStore((s) => s.refreshGitHubCliStatus);
  const subscribeToWorkBuddyEvents = useWorkBuddyStore((s) => s.subscribeToBackgroundEvents);

  useEffect(() => {
    void refreshGitHubCliStatus();
  }, [refreshGitHubCliStatus]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void subscribeToWorkBuddyEvents().then((cleanup) => {
      if (disposed) cleanup();
      else unlisten = cleanup;
    });
    return () => { disposed = true; unlisten?.(); };
  }, [subscribeToWorkBuddyEvents]);

  const ghReady = githubCliStatus?.available && githubCliStatus.authed;

  return (
    <div className="app-container with-sidebar">
      <aside className="sidebar">
        <nav className="sidebar-nav" aria-label="主导航">
          <button
            type="button"
            title="WorkBuddy"
            aria-label="WorkBuddy"
            aria-current={page === 'workbuddy' ? 'page' : undefined}
            className={page === 'workbuddy' ? 'nav-item active' : 'nav-item'}
            onClick={() => setPage('workbuddy')}
          >
            <img src={workBuddyIcon} alt="" width={17} height={17} />
            <span>WorkBuddy</span>
          </button>
          <button
            type="button"
            title="TRAE"
            aria-label="TRAE"
            aria-current={page === 'switcher' ? 'page' : undefined}
            className={page === 'switcher' ? 'nav-item active' : 'nav-item'}
            onClick={() => setPage('switcher')}
          >
            <img src={traeCnIcon} alt="" width={17} height={17} />
            <span>TRAE</span>
          </button>
          <button
            type="button"
            title="豆包"
            aria-label="豆包"
            aria-current={page === 'doubao' ? 'page' : undefined}
            className={page === 'doubao' ? 'nav-item active' : 'nav-item'}
            onClick={() => setPage('doubao')}
          >
            <Bot size={17} />
            <span>豆包</span>
          </button>
          <div className="sidebar-group-label">模型服务</div>
          <button type="button" title="DeepSeek Harness" aria-label="DeepSeek Harness" aria-current={page === 'harness' ? 'page' : undefined} className={page === 'harness' ? 'nav-item harness-nav active' : 'nav-item harness-nav'} onClick={() => setPage('harness')}>
            <Cable size={17} /><span>DeepSeek Harness</span>
          </button>
        </nav>
        <div className="sidebar-footer">
          <div className="app-sidebar-gh" title={ghReady ? 'gh CLI 就绪' : 'gh CLI 未就绪（GitHub 同步与云端签到不可用）'}>
            <span className={ghReady ? 'app-sidebar-gh-dot app-sidebar-gh-dot--ok' : 'app-sidebar-gh-dot'} />
            <span>gh {githubCliStatus ? (ghReady ? '就绪' : '未就绪') : '检测中'}</span>
          </div>
        </div>
      </aside>
      <main className="main-wrapper">
        {page === 'harness' ? <HarnessPage /> : page === 'switcher' ? <WorkCnSwitcherPage /> : page === 'doubao' ? <DoubaoPoolPanel /> : <WorkBuddyPage />}
      </main>
    </div>
  );
}
