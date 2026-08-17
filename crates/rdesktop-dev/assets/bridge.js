// rdesktop bridge script
// This script is injected into the browser during development mode.
// It provides IPC communication and state reporting to the dev server.

(function() {
    'use strict';

    const RDESKTOP_BASE = '/__rdesktop__';

    // IPC bridge: allows frontend to call Rust backend commands
    window.__RDESKTOP_INVOKE__ = async function(cmd, payload) {
        const id = Math.random().toString(36).slice(2);
        const response = await fetch(`${RDESKTOP_BASE}/agent/ipc`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ id, cmd, payload }),
        });
        return response.json();
    };

    // State reporter: sends app state to the dev server
    window.__RDESKTOP_SET_STATE__ = async function(state) {
        await fetch(`${RDESKTOP_BASE}/state`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(state),
        });
    };

    // DOM reporter: sends DOM snapshot to the dev server
    let domReportTimer = null;
    function reportDom() {
        const html = document.documentElement.outerHTML;
        fetch(`${RDESKTOP_BASE}/dom`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ html }),
        }).catch(() => {}); // silent fail
    }

    // Report DOM on changes (debounced)
    let domReportTimeout = null;
    const observer = new MutationObserver(() => {
        clearTimeout(domReportTimeout);
        domReportTimeout = setTimeout(reportDom, 500);
    });

    // Start observing once DOM is ready
    if (document.readyState === 'loading') {
        document.addEventListener('DOMContentLoaded', () => {
            observer.observe(document.body, { childList: true, subtree: true, attributes: true });
            reportDom(); // initial report
        });
    } else {
        observer.observe(document.body, { childList: true, subtree: true, attributes: true });
        reportDom(); // initial report
    }

    // Action executor: polls for pending actions from the agent
    let pendingActions = [];
    async function pollActions() {
        try {
            const resp = await fetch(`${RDESKTOP_BASE}/agent/action/pending`);
            if (resp.ok) {
                const actions = await resp.json();
                for (const action of actions) {
                    executeAction(action);
                }
            }
        } catch (e) {
            // silent fail
        }
    }

    function executeAction(action) {
        const el = document.querySelector(action.selector);
        if (!el) {
            console.warn('[rdesktop] Element not found:', action.selector);
            return;
        }

        switch (action.action) {
            case 'click':
                el.click();
                break;
            case 'double_click':
                el.dispatchEvent(new MouseEvent('dblclick', { bubbles: true }));
                break;
            case 'right_click':
                el.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true }));
                break;
            case 'type':
                el.focus();
                el.value = (el.value || '') + (action.value || '');
                el.dispatchEvent(new Event('input', { bubbles: true }));
                break;
            case 'fill':
                el.focus();
                el.value = action.value || '';
                el.dispatchEvent(new Event('input', { bubbles: true }));
                break;
            case 'clear':
                el.value = '';
                el.dispatchEvent(new Event('input', { bubbles: true }));
                break;
            case 'hover':
                el.dispatchEvent(new MouseEvent('mouseenter', { bubbles: true }));
                break;
            case 'focus':
                el.focus();
                break;
            default:
                console.warn('[rdesktop] Unknown action:', action.action);
        }
    }

    // DevTools overlay (optional)
    if (new URLSearchParams(window.location.search).has('__rdesktop_devtools')) {
        const badge = document.createElement('div');
        badge.style.cssText = 'position:fixed;bottom:8px;right:8px;background:#667eea;color:white;padding:4px 8px;border-radius:4px;font-size:11px;z-index:99999;font-family:monospace;opacity:0.7;';
        badge.textContent = 'rdesktop dev';
        document.body.appendChild(badge);
    }

    console.log('[rdesktop] Bridge script loaded');
})();
