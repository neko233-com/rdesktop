// Only Node's actual consumers retain callback functions. Renderer closures stay
// alive until an acknowledged, generation-checked retirement reaches that page.
function createNodeCallbacks(pack, queue, collectGarbage) {
  const records = new Map(), identities = new WeakMap(), retired = new Map();
  let activity = 0, collectedActivity = -1, lastActivity = Date.now(), registered = 0, lastCollection = 0, overflow = false;
  function touch() { activity++; lastActivity = Date.now(); }
  function retire(record) {
    if (records.get(record.id) !== record) return;
    records.delete(record.id); finalizer.unregister(record);
    pending(record.id, record.epoch);
  }
  function pending(id, epoch) { if (!retired.has(id) && retired.size >= 4096) { overflow = true; return; } retired.set(id, epoch); }
  const finalizer = new FinalizationRegistry(retire);
  function collect(force = false) {
    const now = Date.now(), idle = now - lastActivity >= 1500 && now - lastCollection >= 1500 && activity !== collectedActivity;
    if (!records.size || !(force || (records.size >= 128 && registered >= 64) || idle)) return;
    // Do not dereference WeakRefs before this collection: that would keep every
    // target alive through the current JS job, including an explicit sweep.
    collectGarbage(); lastCollection = now; collectedActivity = activity; registered = 0;
    for (const record of records.values()) if (!record.weak.deref()) retire(record);
  }
  function valid(item) { return item && typeof item.id === 'string' && item.id.length > 0 && item.id.length <= 64 && Number.isSafeInteger(item.epoch) && item.epoch >= 0; }
  return {
    touch, collect,
    claims(items = []) {
      if (!Array.isArray(items) || items.length > 256 || !items.every(valid)) throw new TypeError('Invalid renderer callback claims.');
      // Validate before dispatch; reconcile after it. Callbacks packed into a
      // rejected/incomplete call must not become permanent renderer roots.
      return () => { for (const item of items) if (!records.has(item.id)) pending(item.id, item.epoch); };
    },
    acknowledge(items = []) {
      if (!Array.isArray(items) || items.length > 4096 || !items.every(valid)) throw new TypeError('Invalid callback retirement acknowledgement.');
      for (const item of items) if (retired.get(item.id) === item.epoch) retired.delete(item.id);
    },
    retirements() { if (overflow) throw new RangeError('Callback retirement queue overflowed; close this runtime.'); return [...retired].map(([id, epoch]) => ({ id, epoch })); },
    tag(value) { const record = identities.get(value); return record ? { $rdesktop: 'callback-reference', id: record.id } : null; },
    unpack(value) {
      const epoch = value.epoch ?? 0;
      if (!valid({ id: value.id, epoch })) throw new TypeError('Invalid renderer callback.');
      let record = records.get(value.id), callback = record?.weak.deref();
      if (callback) { record.epoch = epoch; return callback; }
      if (record) retire(record);
      if (records.size >= 256) throw new RangeError('Renderer callback limit reached.');
      record = { id: value.id, epoch };
      callback = function (...args) { touch(); queue({ kind: 'callback', id: record.id, receiver: pack(this), args: args.map(arg => pack(arg)) }); };
      record.weak = new WeakRef(callback); records.set(record.id, record); identities.set(callback, record); finalizer.register(callback, record, record);
      registered++; touch(); return callback;
    },
  };
}
