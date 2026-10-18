// Only hot reload. Keep the real native IPC bridge owned by the renderer.
(() => {
    let generation = Number(document.currentScript?.dataset.rdesktopGeneration);
    if (!Number.isFinite(generation)) generation = undefined;
    let pending = false;
    const timer = setInterval(async () => {
        if (pending) return;
        pending = true;
        try {
            const response = await fetch('/__rdesktop__/reload', {cache: 'no-store'});
            if (!response.ok) return;
            const state = await response.json();
            if (!state.enabled) return;
            if (generation !== undefined && generation !== state.generation) location.reload();
            generation = state.generation;
        } catch (_) { /* The CLI may be restarting its asset server. */ }
        finally { pending = false; }
    }, 300);
    addEventListener('pagehide', () => clearInterval(timer), {once: true});
})();
