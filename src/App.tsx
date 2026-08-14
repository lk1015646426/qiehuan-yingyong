// TRAE Work CN 账号切换器 - 应用入口
//
// 阶段 1：仅渲染 WorkCnSwitcherPage，收敛为 TRAE Work CN 专用壳。
// 旧路由与页面文件暂时保留，仅通过不再引用的方式隐藏，待后续阶段验收后再做物理裁剪。

import './App.css';
import { WorkCnSwitcherPage } from './pages/WorkCnSwitcherPage';

export default function App() {
  return <WorkCnSwitcherPage />;
}
