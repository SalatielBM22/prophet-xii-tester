import type { Command, Metadata } from './types';
export interface Submission { submitted: boolean; acceptedCommandId: number | null }
type Item = { epoch: number; command: Command; commandId: number; resolve: (submission: Submission) => void; reject: (error: Error) => void };
/** Bounded, ordered HTTP control lane. Pointer updates coalesce before submission. */
export class ControlTransport {
  private ownHostCommands = new Set<number>();
  private sequence = 0;
  private parameters = new Map<number, Item>();
  private queue: Item[] = [];
  private timer?: ReturnType<typeof setTimeout>;
  private active: AbortController | null = null;
  private running = false;
  private disposed = false;
  private epoch = 0;
  metadata: Metadata = { protocolVersion: 1, capabilities: [], presets: [] };
  constructor(private token: string, private request: typeof fetch = (...args) => fetch(...args)) {}
  get generation() { return this.epoch; }
  get auth() { return this.token ? `?token=${encodeURIComponent(this.token)}` : ''; }
  get supportsAck() { return this.metadata.capabilities.includes('parameterAck'); }
  isOwnHostCommand(id: number) { return this.ownHostCommands.has(id); }
  nextId() { return ++this.sequence; }
  send(command: Command, commandId = this.nextId()): Promise<Submission> {
    if (this.disposed) return Promise.reject(Error('Control connection closed'));
    if (['cancel_learn', 'unlearn', 'remap'].includes(command.type) && !this.metadata.capabilities.includes('midiLearnLifecycle')) return Promise.reject(Error('MIDI Learn lifecycle unsupported by host'));
    if (command.type === 'reverb_bypass' && !this.metadata.capabilities.includes('reverbBypass')) return Promise.reject(Error('Reverb bypass unsupported by host'));
    return new Promise((resolve, reject) => {
      const item = { epoch: this.epoch, command, commandId, resolve, reject };
      if (command.type === 'param') {
        this.parameters.get(command.id)?.resolve({ submitted: false, acceptedCommandId: null });
        this.parameters.set(command.id, item);
        if (!this.timer) this.timer = setTimeout(() => { this.timer = undefined; this.flushParameters(); void this.drain(); }, 16);
      } else {
        this.flushParameters();
        if (this.queue.length >= 128) { reject(Error('Control queue full')); return; }
        this.queue.push(item);
        void this.drain();
      }
    });
  }
  private flushParameters() {
    const batch = [...this.parameters.values()].sort((a, b) => a.commandId - b.commandId);
    for (const item of batch) {
      // Never coalesce across a non-parameter command: snapshots/presets/panic
      // are ordering barriers. Remove then append so cumulative ACKs stay causal.
      let barrier = -1;
      for (let i = this.queue.length - 1; i >= 0; i--) if (this.queue[i].command.type !== 'param') { barrier = i; break; }
      const index = this.queue.findIndex((x, i) => i > barrier && x.command.type === 'param' && item.command.type === 'param' && x.command.id === item.command.id);
      if (index >= 0) this.queue.splice(index, 1)[0].resolve({ submitted: false, acceptedCommandId: null });
      if (this.queue.length < 128) this.queue.push(item);
      else item.reject(Error('Control queue full'));
    }
    this.parameters.clear();
  }
  private async drain() {
    if (this.running || this.disposed) return;
    this.running = true;
    try {
      while (this.queue.length && !this.disposed) {
        const item = this.queue.shift()!;
        this.active = new AbortController();
        const timeout = setTimeout(() => this.active?.abort(), 5000);
        try {
          const body = item.command;
          const response = await this.request('/api/v1/command' + this.auth, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body), signal: this.active.signal });
          if (!response.ok) throw Error(`Control returned ${response.status}`);
          if (item.epoch !== this.epoch) throw Error('Stale control session');
          const text = await response.text();
          let acceptedCommandId: number | null = null;
          if (text.trim()) {
            const payload = JSON.parse(text) as { acceptedCommandId?: unknown };
            if (typeof payload.acceptedCommandId !== 'number' || !Number.isSafeInteger(payload.acceptedCommandId) || payload.acceptedCommandId <= 0) throw Error('Invalid command acceptance reply');
            acceptedCommandId = payload.acceptedCommandId;
          }
          if (this.supportsAck && acceptedCommandId === null) throw Error('Native host omitted acceptedCommandId');
          if (item.epoch !== this.epoch) throw Error('Stale control session');
          if (acceptedCommandId !== null) {
            this.ownHostCommands.add(acceptedCommandId);
            if (this.ownHostCommands.size > 512) this.ownHostCommands.delete(this.ownHostCommands.values().next().value!);
          }
          item.resolve({ submitted: true, acceptedCommandId });
        } catch (error) { item.reject(error instanceof Error ? error : Error(String(error))); }
        finally { clearTimeout(timeout); this.active = null; }
      }
    } finally { this.running = false; }
  }
  reset() {
    this.epoch++;
    this.ownHostCommands.clear();
    clearTimeout(this.timer);
    this.timer = undefined;
    this.active?.abort();
    for (const item of [...this.queue, ...this.parameters.values()]) item.reject(Error('Control connection closed'));
    this.queue = [];
    this.parameters.clear();
  }
  dispose() { this.disposed = true; this.reset(); }
}
