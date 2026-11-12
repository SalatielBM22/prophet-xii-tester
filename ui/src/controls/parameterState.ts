import specs from '../param-specs.json';
import type { Frame } from '../protocol/types';
import { ControlTransport } from '../protocol/transport';
import { normalize } from './parameters';
type Pending = { value: number; commandId: number; accepted: boolean; since: number };
type Edit = { id: number; before: number; after: number };
export interface ParameterSnapshot { values: number[]; pending: number; canUndo: boolean; canRedo: boolean; error: string }
/** Local intent remains visible until echoed (v1 heuristic) or acknowledged (v2). */
export class ParameterState {
  private current: ParameterSnapshot = { values: specs.map(s => s.min), pending: 0, canUndo: false, canRedo: false, error: '' };
  private listeners = new Set<() => void>();
  private pending = new Map<number, Pending>();
  private history: Edit[][] = [];
  private future: Edit[][] = [];
  private gesture: { before: number[]; ids: Set<number> } | null = null;
  private authoritative = [...this.current.values];
  private lastFrame = -1;
  private lastSequence = -1;
  private lastRevision = -1;
  private sessionId?: string;
  private epoch = 0;
  constructor(readonly transport: ControlTransport, private now: () => number = Date.now) {}
  subscribe = (listener: () => void) => { this.listeners.add(listener); return () => this.listeners.delete(listener); };
  getSnapshot = () => this.current;
  private publish(values = this.current.values, error = this.current.error) {
    this.current = { values, pending: this.pending.size, canUndo: this.history.length > 0, canRedo: this.future.length > 0, error };
    this.listeners.forEach(listener => listener());
  }
  begin = (id: number) => {
    if (!this.gesture) this.gesture = { before: [...this.current.values], ids: new Set() };
    this.gesture.ids.add(id);
  };
  end = () => {
    if (!this.gesture) return;
    const edits = [...this.gesture.ids].map(id => ({ id, before: this.gesture!.before[id], after: this.current.values[id] })).filter(x => x.before !== x.after);
    this.gesture = null;
    if (edits.length) { this.history.push(edits); this.history = this.history.slice(-50); this.future = []; }
    this.publish();
  };
  edit = (id: number, value: number) => {
    value = normalize(specs[id], value);
    if (this.current.values[id] === value) return;
    const atomic = !this.gesture;
    if (atomic) this.begin(id);
    this.gesture?.ids.add(id);
    this.submit(id, value);
    if (atomic) this.end();
  };
  private submit(id: number, value: number) {
    const epoch = this.epoch;
    const commandId = this.transport.nextId();
    this.pending.set(id, { value, commandId, accepted: false, since: this.now() });
    const values = [...this.current.values]; values[id] = value;
    this.publish(values, '');
    void this.transport.send({ type: 'param', id, value }, commandId).then(submitted => {
      if (epoch !== this.epoch) return;
      const pending = this.pending.get(id);
      if (submitted && pending?.commandId === commandId) pending.accepted = true;
    }).catch(error => {
      if (epoch !== this.epoch || this.pending.get(id)?.commandId !== commandId) return;
      this.pending.delete(id);
      const next = [...this.current.values]; next[id] = this.authoritative[id];
      this.publish(next, String(error));
    });
  }
  receive = (frame: Frame) => {
    if (frame.v === 2) {
      if (this.sessionId !== undefined && frame.sessionId !== this.sessionId) this.disconnect();
      this.sessionId = frame.sessionId;
      if (frame.sequence! <= this.lastSequence || frame.parameterRevision! < this.lastRevision) return false;
      this.lastSequence = frame.sequence!;
      this.lastRevision = frame.parameterRevision!;
    } else if (frame.frames <= this.lastFrame) return false;
    this.lastFrame = frame.frames;
    this.authoritative = [...frame.values];
    const values = frame.values.map((value, id) => {
      const pending = this.pending.get(id);
      if (pending) {
        const acknowledged = frame.v === 2 && this.transport.supportsAck
          ? frame.clientId === this.transport.clientId && Number(frame.appliedCommandId) >= pending.commandId
          : pending.accepted && Math.abs(value - pending.value) <= Math.max(1e-5, Math.abs(pending.value) * 1e-6);
        if (acknowledged && !this.gesture?.ids.has(id)) this.pending.delete(id);
        else return pending.value;
      }
      return this.gesture?.ids.has(id) ? this.current.values[id] : value;
    });
    if (values.some((value, id) => value !== this.current.values[id]) || this.current.pending !== this.pending.size) this.publish(values);
    return true;
  };
  expire = () => {
    const expired = [...this.pending].filter(([, pending]) => this.now() - pending.since > 4000);
    if (!expired.length) return;
    const values = [...this.current.values];
    for (const [id] of expired) { this.pending.delete(id); values[id] = this.authoritative[id]; }
    this.gesture = null;
    this.publish(values, 'Host did not confirm controls within 4 seconds; restored latest host values');
  };
  disconnect = () => {
    this.epoch++;
    this.pending.clear(); this.gesture = null;
    this.lastFrame = -1; this.lastSequence = -1; this.lastRevision = -1; this.sessionId = undefined;
    this.publish([...this.authoritative], '');
  };
  acceptPresetBarrier = (commandId: number) => {
    for (const [id, pending] of this.pending) if (pending.commandId < commandId) this.pending.delete(id);
    this.gesture = null; this.history = []; this.future = [];
    this.publish(this.authoritative.map((value, id) => this.pending.get(id)?.value ?? value));
  };
  undo = () => {
    this.end(); const edits = this.history.pop(); if (!edits) return;
    this.future.push(edits);
    for (const edit of edits) this.submit(edit.id, edit.before);
    this.publish();
  };
  redo = () => {
    this.end(); const edits = this.future.pop(); if (!edits) return;
    this.history.push(edits);
    for (const edit of edits) this.submit(edit.id, edit.after);
    this.publish();
  };
  applySnapshot = (values: number[]) => {
    if (values.length !== specs.length || values.some((v, i) => !Number.isFinite(v) || normalize(specs[i], v) !== v)) throw Error('Invalid preset values');
    this.end();
    this.begin(0);
    for (let id = 0; id < values.length; id++) if (values[id] !== this.current.values[id]) this.edit(id, values[id]);
    this.end();
  };
}
