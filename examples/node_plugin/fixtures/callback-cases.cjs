// Same contract in an independent Node reference and the actual native document.
module.exports = async function callbackCases(require, directory, progress = () => {}, probeBudget = () => {}) {
  const fs = require('node:fs'), path = require('node:path');
  const { EventEmitter } = require('node:events');
  const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
  const target = path.join(directory, 'callback-owned.txt'); fs.writeFileSync(target, 'callback 中文');
  const checks = {}, emitter = new EventEmitter(); emitter.setMaxListeners(0);
  let receiver = true; const hits = [];
  function retained(value) { receiver &&= this === emitter; hits.push(value); }
  emitter.on('owned', retained); emitter.emit('owned', 'before');
  let completed = 0;
  for (let index = 0; index < 600; index++) {
    const value = await new Promise((resolve, reject) => fs.readFile(target, 'utf8', (error, value) => error ? reject(error) : resolve(value)));
    if (value !== 'callback 中文') throw new Error('Callback result changed');
    progress(++completed);
  }
  checks.sequentialCallbacks = completed;
  emitter.emit('owned', 'after'); await pause(60);
  checks.retainedListenerSurvives = hits.join(',') === 'before,after';
  checks.callbackReceiver = receiver;
  checks.callbackRoundTripIdentity = emitter.listeners('owned')[0] === retained;
  emitter.off('owned', retained); checks.removeListenerIdentity = emitter.listenerCount('owned') === 0;
  await pause(2000);
  emitter.on('owned', retained); emitter.emit('owned', 'again'); await pause(60);
  checks.reusedFunctionAfterRetirement = hits.join(',') === 'before,after,again';
  emitter.off('owned', retained);
  let once = 0; emitter.once('single', () => once++); emitter.emit('single'); emitter.emit('single'); await pause(60);
  checks.onceRemovesBeforeSecondEmission = once === 1 && emitter.listenerCount('single') === 0;
  await pause(2000);
  // Retaining actual listeners must still exhaust the simultaneous live budget.
  // The independent reference has no framework budget, so it records semantics
  // around the same number of retained handlers without changing Node's limits.
  for (let index = 0; index < 256; index++) emitter.on('budget', () => {});
  checks.retainedBudget = emitter.listenerCount('budget');
  probeBudget(emitter);
  emitter.removeAllListeners('budget');
  const done = await new Promise(resolve => { emitter.once('post-budget', resolve); emitter.emit('post-budget', 'usable'); });
  checks.usableAfterListenerRemoval = done;
  checks.workerFlagsHidden = require('node:process').execArgv.length === 0;
  return checks;
};
if (require.main === module) module.exports(require, __dirname).then(value => process.stdout.write(JSON.stringify(value)), error => { console.error(error); process.exitCode = 1; });
