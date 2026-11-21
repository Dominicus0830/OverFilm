import React from 'react';
import {createRoot} from 'react-dom/client';
import {Workspace} from './Workspace';
import '../styles.css';
import './workspace.css';

// 시안 전용 입구(mockup.html). 서버 없이 가짜 데이터로 그린다.
// 주소 뒤 ?theme=light, ?view=log|learn|passed|collapsed 로 캡처할 상태를 고른다.
const q = new URLSearchParams(location.search);
document.documentElement.dataset.theme = q.get('theme') === 'light' ? 'light' : 'dark';
const el = document.getElementById('root');
if (el) createRoot(el).render(<React.StrictMode><Workspace view={q.get('view') ?? ''} /></React.StrictMode>);
