// rdesktop bridge script
// Injected into every HTML document served by `rdesktop dev`.
// It provides IPC, agent actions, hot reload, and the single visual recorder.

(function() {
    'use strict';

    const RDESKTOP_BASE = '/__rdesktop__';
    const RECORDING_BASE = `${RDESKTOP_BASE}/agent/recording`;

    async function postJson(url, body, options) {
        const response = await fetch(url, Object.assign({
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(body || {}),
        }, options || {}));
        const json = await response.json().catch(() => ({}));
        if (!response.ok) {
            throw new Error(json.error || `rdesktop request failed (${response.status})`);
        }
        return json;
    }

    // IPC bridge: allows frontend code to call development commands.
    window.__RDESKTOP_INVOKE__ = async function(cmd, payload) {
        const id = Math.random().toString(36).slice(2);
        return postJson(`${RDESKTOP_BASE}/agent/ipc`, { id, cmd, payload });
    };

    // State reporter: sends app state to the dev server.
    window.__RDESKTOP_SET_STATE__ = async function(state) {
        await postJson(`${RDESKTOP_BASE}/state`, state || {});
    };

    // DOM reporter: sends DOM snapshots to the dev server.
    function reportDom() {
        const html = document.documentElement.outerHTML;
        postJson(`${RDESKTOP_BASE}/dom`, { html }).catch(() => {});
    }

    let domReportTimeout = null;
    const observer = new MutationObserver(() => {
        clearTimeout(domReportTimeout);
        domReportTimeout = setTimeout(reportDom, 500);
    });

    function startDomReporting() {
        if (!document.body) return;
        observer.observe(document.body, { childList: true, subtree: true, attributes: true });
        reportDom();
    }
    if (document.readyState === 'loading') {
        document.addEventListener('DOMContentLoaded', startDomReporting, { once: true });
    } else {
        startDomReporting();
    }

    // Agent action executor.
    async function pollActions() {
        try {
            const response = await fetch(`${RDESKTOP_BASE}/agent/action/pending`);
            if (!response.ok) return;
            const actions = await response.json();
            for (const action of actions) executeAction(action);
        } catch (_) {
            // The dev server may be restarting during a hot reload.
        }
    }

    async function reportActionResult(action, success, error) {
        if (!action.id) return;
        await postJson(`${RDESKTOP_BASE}/agent/action/result`, {
            id: action.id,
            success,
            error: error ? String(error) : null,
            side_effects: success ? ['bridge action applied'] : [],
        }).catch(() => {});
    }

    function executeAction(action) {
        const el = action.selector ? document.querySelector(action.selector) : null;
        if (!el && !['scroll', 'press'].includes(action.action)) {
            console.warn('[rdesktop] Element not found:', action.selector);
            void reportActionResult(action, false, `Element not found: ${action.selector}`);
            return;
        }

        function dispatchPointer(target, type, point, buttons) {
            if (!target) return;
            const init = {
                bubbles: true,
                cancelable: true,
                view: window,
                clientX: point.x,
                clientY: point.y,
                button: 0,
                buttons: buttons || 0,
                pointerId: 1,
                pointerType: 'mouse',
                isPrimary: true,
            };
            const PointerCtor = window.PointerEvent || window.MouseEvent;
            target.dispatchEvent(new PointerCtor(type, init));
        }

        function dispatchMouse(target, type, point, buttons) {
            if (!target) return;
            target.dispatchEvent(new MouseEvent(type, {
                bubbles: true,
                cancelable: true,
                view: window,
                clientX: point.x,
                clientY: point.y,
                button: 0,
                buttons: buttons || 0,
            }));
        }

        function centerOf(target) {
            const rect = target.getBoundingClientRect();
            return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
        }

        function dispatchDragEvent(target, type, point, dataTransfer) {
            if (!target) return;
            const init = {
                bubbles: true,
                cancelable: true,
                clientX: point.x,
                clientY: point.y,
                dataTransfer,
            };
            if (window.DragEvent) {
                target.dispatchEvent(new DragEvent(type, init));
            } else {
                target.dispatchEvent(new CustomEvent(type, { bubbles: true, cancelable: true, detail: init }));
            }
        }

        function drag(source, action) {
            const target = action.target_selector ? document.querySelector(action.target_selector) : source;
            if (!target) {
                throw new Error(`Drag target not found: ${action.target_selector}`);
            }
            const from = action.from ? { x: action.from[0], y: action.from[1] } : centerOf(source);
            const to = action.to ? { x: action.to[0], y: action.to[1] } : centerOf(target);
            const duration = Math.max(0, Math.min(1000, Number(action.duration_ms || 0)));
            const steps = Math.max(2, Math.min(30, Math.ceil(duration / 16) || 6));
            const dataTransfer = typeof DataTransfer === 'function' ? new DataTransfer() : null;

            dispatchPointer(source, 'pointerover', from, 0);
            dispatchMouse(source, 'mouseover', from, 0);
            dispatchDragEvent(source, 'dragstart', from, dataTransfer);
            dispatchPointer(source, 'pointerdown', from, 1);
            dispatchMouse(source, 'mousedown', from, 1);
            for (let index = 1; index <= steps; index += 1) {
                const ratio = index / steps;
                const point = {
                    x: from.x + (to.x - from.x) * ratio,
                    y: from.y + (to.y - from.y) * ratio,
                };
                dispatchPointer(document, 'pointermove', point, 1);
                dispatchMouse(document, 'mousemove', point, 1);
                dispatchDragEvent(target, 'dragover', point, dataTransfer);
            }
            dispatchDragEvent(target, 'drop', to, dataTransfer);
            dispatchPointer(target, 'pointerup', to, 0);
            dispatchMouse(target, 'mouseup', to, 0);
            dispatchDragEvent(source, 'dragend', to, dataTransfer);
        }

        function press(target, value) {
            const key = value || 'Enter';
            const eventInit = { key, code: key, bubbles: true, cancelable: true };
            const receiver = target || document.activeElement || document;
            receiver.dispatchEvent(new KeyboardEvent('keydown', eventInit));
            receiver.dispatchEvent(new KeyboardEvent('keyup', eventInit));
        }

        try {
            switch (action.action) {
                case 'click': el.click(); break;
                case 'double_click': el.dispatchEvent(new MouseEvent('dblclick', { bubbles: true })); break;
                case 'right_click': el.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true })); break;
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
                    el.focus();
                    el.value = '';
                    el.dispatchEvent(new Event('input', { bubbles: true }));
                    break;
                case 'scroll':
                    (el || window).scrollBy((action.coordinates || [0, 0])[0], (action.coordinates || [0, 0])[1]);
                    break;
                case 'hover':
                    dispatchPointer(el, 'pointerover', centerOf(el), 0);
                    dispatchMouse(el, 'mouseover', centerOf(el), 0);
                    el.dispatchEvent(new MouseEvent('mouseenter', { bubbles: true }));
                    break;
                case 'focus': el.focus(); break;
                case 'select':
                    el.value = action.value || '';
                    el.dispatchEvent(new Event('change', { bubbles: true }));
                    break;
                case 'drag': drag(el, action); break;
                case 'press': press(el, action.value); break;
                default: throw new Error(`Unknown action: ${action.action}`);
            }
            void reportActionResult(action, true, null);
        } catch (error) {
            console.warn('[rdesktop] Action failed:', error);
            void reportActionResult(action, false, error && error.message ? error.message : error);
        }
    }

    // Hot reload is version based, so it works for plain static frontend files
    // without requiring a websocket client or a bundler-specific runtime.
    let reloadGeneration = null;
    async function pollReload() {
        try {
            const response = await fetch(`${RDESKTOP_BASE}/reload`, { cache: 'no-store' });
            if (!response.ok) return;
            const state = await response.json();
            if (reloadGeneration !== null && state.generation !== reloadGeneration) {
                window.location.reload();
                return;
            }
            reloadGeneration = state.generation;
        } catch (_) {
            // The server may be compiling or restarting.
        }
    }

    // Browser fallback recorder. On Windows the server owns native desktop
    // capture and this state stays unused; in either backend the server is the
    // authority, so a page reload cannot create a second file.
    const recorderState = {
        sessionId: null,
        recorder: null,
        stream: null,
        uploadChain: Promise.resolve(),
        starting: false,
        lastErrorAt: 0,
    };

    function supportedMimeType() {
        if (!window.MediaRecorder || !MediaRecorder.isTypeSupported) return null;
        const candidates = [
            'video/mp4;codecs=avc1.42E01E',
            'video/mp4',
            'video/webm;codecs=vp9,opus',
            'video/webm;codecs=vp8,opus',
            'video/webm',
        ];
        return candidates.find((mime) => MediaRecorder.isTypeSupported(mime)) || null;
    }

    async function reportRecordingError(sessionId, error) {
        try {
            await postJson(`${RECORDING_BASE}/error`, {
                session_id: sessionId,
                error: String(error && error.message ? error.message : error),
            });
        } catch (_) {}
    }

    async function uploadRecordingChunk(sessionId, blob) {
        const response = await fetch(`${RECORDING_BASE}/chunk`, {
            method: 'POST',
            headers: { 'X-Rdesktop-Recording-Id': sessionId },
            body: blob,
        });
        if (!response.ok) {
            const json = await response.json().catch(() => ({}));
            throw new Error(json.error || `recording chunk failed (${response.status})`);
        }
    }

    async function requestDisplayStream() {
        if (!navigator.mediaDevices || !navigator.mediaDevices.getDisplayMedia) {
            throw new Error('display capture is not supported by this browser');
        }
        return navigator.mediaDevices.getDisplayMedia({
            video: { frameRate: 30, cursor: 'always' },
            audio: false,
        });
    }

    async function stopBrowserRecorder(sessionId) {
        if (!recorderState.recorder || recorderState.sessionId !== sessionId) return;
        const recorder = recorderState.recorder;
        if (recorder.state === 'recording' || recorder.state === 'paused') {
            recorder.stop();
        }
    }

    async function startBrowserRecorder(recording, providedStream) {
        if (recorderState.starting && !providedStream) return;
        if (recorderState.recorder && recorderState.sessionId === recording.session_id) return;
        if (!recording.session_id) return;
        if (Date.now() - recorderState.lastErrorAt < 4000) return;

        recorderState.starting = true;
        const sessionId = recording.session_id;
        try {
            // The browser may ask the user to select this tab/window. Once the
            // permission is granted, subsequent idempotent starts reuse the same
            // server session and do not create another output file.
            const stream = providedStream || await requestDisplayStream();
            const mimeType = supportedMimeType();
            if (!mimeType) throw new Error('this browser cannot encode MP4 or WebM with MediaRecorder');

            const recorder = new MediaRecorder(stream, {
                mimeType,
                videoBitsPerSecond: 5_000_000,
            });
            recorderState.sessionId = sessionId;
            recorderState.stream = stream;
            recorderState.recorder = recorder;
            recorderState.uploadChain = Promise.resolve();

            await postJson(`${RECORDING_BASE}/started`, {
                session_id: sessionId,
                mime_type: mimeType,
            });

            recorder.ondataavailable = (event) => {
                if (!event.data || event.data.size === 0) return;
                recorderState.uploadChain = recorderState.uploadChain.then(() =>
                    uploadRecordingChunk(sessionId, event.data)
                );
                recorderState.uploadChain.catch((error) => reportRecordingError(sessionId, error));
            };
            recorder.onerror = (event) => {
                reportRecordingError(sessionId, event.error || 'MediaRecorder error');
            };
            recorder.onstop = async () => {
                try {
                    await recorderState.uploadChain;
                    await postJson(`${RECORDING_BASE}/complete`, {
                        session_id: sessionId,
                        mime_type: mimeType,
                    });
                } catch (error) {
                    await reportRecordingError(sessionId, error);
                } finally {
                    stream.getTracks().forEach((track) => track.stop());
                    if (recorderState.sessionId === sessionId) {
                        recorderState.sessionId = null;
                        recorderState.recorder = null;
                        recorderState.stream = null;
                    }
                }
            };
            recorder.start(1000);
        } catch (error) {
            recorderState.lastErrorAt = Date.now();
            if (recorderState.stream) recorderState.stream.getTracks().forEach((track) => track.stop());
            recorderState.sessionId = null;
            recorderState.recorder = null;
            recorderState.stream = null;
            await reportRecordingError(sessionId, error);
        } finally {
            recorderState.starting = false;
        }
    }

    async function pollRecording() {
        try {
            const response = await fetch(`${RECORDING_BASE}/poll`, { cache: 'no-store' });
            if (!response.ok) return;
            const recording = await response.json();
            if (recording.status === 'recording' && recording.session_id && !recording.native) {
                await startBrowserRecorder(recording);
            } else if (recording.status === 'stop_requested' && recording.session_id && !recording.native) {
                await stopBrowserRecorder(recording.session_id);
            }
        } catch (_) {
            // The dev server may be restarting during hot reload.
        }
    }

    // These helpers are useful when an agent or a human wants to start from a
    // click handler, which satisfies browsers that require user activation for
    // getDisplayMedia(). The HTTP API remains the source of truth.
    window.__RDESKTOP_START_RECORDING__ = async function() {
        // Request the stream before awaiting HTTP so the browser's user
        // activation is still valid when this is called by a real click.
        if (recorderState.starting) return { recording: { mime_type: 'starting' } };
        recorderState.starting = true;
        let stream;
        try {
            const current = await fetch(`${RECORDING_BASE}`, { cache: 'no-store' }).then((response) => {
                if (!response.ok) throw new Error(`recording status failed (${response.status})`);
                return response.json();
            });
            // Windows dev servers capture the native desktop and encode MP4
            // directly. No browser permission dialog or duplicate recorder is
            // needed for this backend.
            if (current.native) return await postJson(`${RECORDING_BASE}/start`, {});

            stream = await requestDisplayStream();
            const response = await postJson(`${RECORDING_BASE}/start`, {});
            await startBrowserRecorder(response.recording, stream);
            return response;
        } catch (error) {
            if (stream) stream.getTracks().forEach((track) => track.stop());
            recorderState.starting = false;
            throw error;
        }
    };
    window.__RDESKTOP_STOP_RECORDING__ = (sessionId) => postJson(`${RECORDING_BASE}/stop`, sessionId ? { session_id: sessionId } : {});

    setInterval(pollActions, 250);
    setInterval(pollReload, 400);
    setInterval(pollRecording, 400);
    pollActions();
    pollReload();
    pollRecording();

    function installDevtools() {
      if (new URLSearchParams(window.location.search).has('__rdesktop_devtools') && document.body) {
        const badge = document.createElement('div');
        badge.style.cssText = 'position:fixed;bottom:8px;right:8px;background:#667eea;color:white;padding:4px 8px;border-radius:4px;font-size:11px;z-index:99999;font-family:monospace;opacity:0.7;';
        badge.textContent = 'rdesktop dev';
        document.body && document.body.appendChild(badge);

        const controls = document.createElement('div');
        controls.style.cssText = 'position:fixed;bottom:36px;right:8px;background:rgba(20,24,38,.92);color:#fff;padding:8px;border-radius:6px;z-index:99999;font:12px monospace;display:flex;gap:6px;align-items:center;';
        const startButton = document.createElement('button');
        startButton.id = 'rdesktop-recording-start';
        startButton.textContent = 'Record';
        const stopButton = document.createElement('button');
        stopButton.id = 'rdesktop-recording-stop';
        stopButton.textContent = 'Stop';
        const label = document.createElement('span');
        label.textContent = 'idle';
        startButton.onclick = async () => {
            label.textContent = 'starting…';
            try {
                const result = await window.__RDESKTOP_START_RECORDING__();
                label.textContent = result.recording.mime_type || 'recording';
            } catch (error) {
                label.textContent = `error: ${error.message}`;
            }
        };
        stopButton.onclick = async () => {
            try {
                await window.__RDESKTOP_STOP_RECORDING__();
                label.textContent = 'stopping…';
            } catch (error) {
                label.textContent = `error: ${error.message}`;
            }
        };
        controls.append(startButton, stopButton, label);
        document.body.appendChild(controls);
      }
    }
    if (document.readyState === 'loading') {
        document.addEventListener('DOMContentLoaded', installDevtools, { once: true });
    } else {
        installDevtools();
    }

    console.log('[rdesktop] Bridge script loaded');
})();
