// 切换应用 - 应用入口
//
// 侧边栏双面板布局（复用上游 pages/sidebar.css 的 .with-sidebar 体系）：
// - TRAE：WorkCnSwitcherPage（账号快照管理 / 一键切换 / 云端签到）
// - WorkBuddy：WorkBuddyPage（账号管理 / 签到 / 积分）
// 页签为前端 state，不引入路由库。

import './App.css';
import { useEffect, useState } from 'react';
import traeCnIcon from './assets/icons/trae-cn.png';
import workBuddyIcon from './assets/icons/workbuddy.png';
import { WorkCnSwitcherPage } from './pages/WorkCnSwitcherPage';
import { WorkBuddyPage } from './pages/WorkBuddyPage';
import { useWorkCnStore } from './stores/useWorkCnStore';
import { useWorkBuddyStore } from './stores/useWorkBuddyStore';

type PageKey = 'switcher' | 'workbuddy';

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
        <div className="sidebar-header">
          <div className="sidebar-brand">
            <img src={traeCnIcon} alt="TRAE" width={22} height={22} />
            <h1>切换应用</h1>
          </div>
        </div>
        <nav className="sidebar-nav">
          <button
            type="button"
            className={page === 'workbuddy' ? 'nav-item active' : 'nav-item'}
            onClick={() => setPage('workbuddy')}
          >
            <img src={workBuddyIcon} alt="" width={17} height={17} />
            <span>WorkBuddy</span>
          </button>
          <button
            type="button"
            className={page === 'switcher' ? 'nav-item active' : 'nav-item'}
            onClick={() => setPage('switcher')}
          >
            <img src={traeCnIcon} alt="" width={17} height={17} />
            <span>TRAE</span>
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
        {page === 'switcher' ? <WorkCnSwitcherPage /> : <WorkBuddyPage />}
      </main>
    </div>
  );
}
