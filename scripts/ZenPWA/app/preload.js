'use strict';

const { contextBridge, ipcRenderer } = require('electron');

const listeners = new Map();
const allowedEvents = new Set([
  'pointer',
  'focus',
  'fullscreen',
  'cmd',
  'widevine',
  'toast',
  'window-state',
  'tint',
]);

function on(channel, callback) {
  if (!allowedEvents.has(channel)) return () => {};
  const wrapped = (_event, ...args) => callback(...args);
  ipcRenderer.on(channel, wrapped);
  const previous = listeners.get(channel);
  const entry = () => ipcRenderer.removeListener(channel, wrapped);
  listeners.set(channel, entry);
  return () => {
    entry();
    if (previous && previous !== entry) previous();
  };
}

contextBridge.exposeInMainWorld('zen', {
  init: () => ipcRenderer.invoke('site:init'),
  windowAction: (action, value) => ipcRenderer.invoke('win:action', action, value),
  setTitle: (title) => ipcRenderer.send('win:title', String(title || '')),
  setButtons: (visible) => ipcRenderer.send('win:buttons', Boolean(visible)),
  faviconAccent: (urls) => ipcRenderer.invoke('accent:favicon', urls),
  copy: (text) => ipcRenderer.invoke('clipboard:write', String(text || '')),
  openInZen: (url) => ipcRenderer.invoke('open:zen', String(url || '')),
  openExternal: (url) => ipcRenderer.invoke('open:external', String(url || '')),
  checkWidevine: () => ipcRenderer.invoke('widevine:check'),
  signWidevine: () => ipcRenderer.invoke('widevine:sign'),
  showMenu: (anchor) => ipcRenderer.invoke('menu:show', anchor || null),
  notify: (message, kind) => ipcRenderer.send('toast', { message: String(message || ''), kind: kind || 'info' }),
  on,
});
