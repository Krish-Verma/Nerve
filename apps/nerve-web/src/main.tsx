/**
 * Entry point.
 *
 * Nothing happens here except mounting. The document this attaches to is compiled into the
 * `nerve` binary as fixed bytes, so there is no server-side rendering, no hydration and no
 * template — the page is inert until this file runs and every pixel after that is React's.
 */

import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';

import { App } from './App';
import './styles/nerve.css';

const host = document.getElementById('root');
if (host === null) {
  throw new Error('the served document is missing its #root element');
}

createRoot(host).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
