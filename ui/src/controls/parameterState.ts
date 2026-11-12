import specs from '../param-specs.json';
import type { Frame } from '../protocol/types';
import { ControlTransport } from '../protocol/transport';
import type { Submission } from '../protocol/transport';
import { normalize } from './parameters';
type Pending = { value: number; commandId: number; accepted: boolean; acceptedCommandId: number | null; since: number };
type Edit = { id: number; before: number; after: number };
export interface ParameterSnapshot { values: number[]; pending: number; canUndo: boolean; canRedo: boolean; error: string }
/** Native acceptance is enqueue only; application is confirmed per parameter. */
export class ParameterState {
  private current: ParameterSnapshot = { values: specs.map(s => s.min), pending: 0, canUndo: false, canRedo: false, error: '' };
  private listeners = new Set<() => void>();
  private pending = new Map<number, Pending>();
  private history: Edit[][] = [];
  private future: Edit[][] = [];
  private gesture: { before: number[]; ids: Set<number> } | null = null;
  private authoritative = [...this.current.values];
  private latest: Frame | null = null;
  private lastFrame = -1;
  private lastSequence = -1;
  private lastRevision = -1;
  private lastMonotonicNs = -1;
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
    this.reconcile();
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
  private applySubmission(commandId: number, submission: Submission) {
    if (!submission.submitted) return;
    for (const pending of this.pending.values()) if (pending.commandId === commandId) {
      pending.accepted = true; pending.acceptedCommandId = submission.acceptedCommandId;
    }
    this.reconcile();
  }
  private submit(id: number, value: number) {
    const epoch = this.epoch, commandId = this.transport.nextId();
    this.pending.set(id, { value, commandId, accepted: false, acceptedCommandId: null, since: this.now() });
    const values = [...this.current.values]; values[id] = value;
    this.publish(values, '');
    void this.transport.send({ type: 'param', id, value }, commandId).then(submission => {
      if (epoch === this.epoch) this.applySubmission(commandId, submission);
    }).catch(error => {
      if (epoch !== this.epoch || this.pending.get(id)?.commandId !== commandId) return;
      this.pending.delete(id);
      const next = [...this.current.values]; next[id] = this.authoritative[id];
      this.publish(next, String(error));
    });
  }
  private reconcile() {
    const frame = this.latest; if (!frame) return;
    let error = this.current.error;
    const values = frame.values.map((value, id) => {
      const pending = this.pending.get(id);
      if (pending) {
        let applied = false, rejected = false;
        if (frame.parameterCommandIds && this.transport.supportsAck) {
          const acceptedId = pending.acceptedCommandId;
          if (acceptedId !== null) {
            applied = frame.parameterCommandIds[id] === acceptedId;
            if (frame.control?.lastDroppedCommandId === acceptedId) { rejected = true; error = `Host dropped the command for ${specs[id].name}`; }
            else if (frame.parameterCommandIds[id] > acceptedId) { rejected = true; error = `${specs[id].name} was superseded by another host command`; }
          }
        } else if (!this.transport.supportsAck) {
          applied = pending.accepted && Math.abs(value - pending.value) <= Math.max(1e-5, Math.abs(pending.value) * 1e-6);
        }
        if ((applied && !this.gesture?.ids.has(id)) || rejected) this.pending.delete(id);
        else return pending.value;
      }
      return this.gesture?.ids.has(id) ? this.current.values[id] : value;
    });
    if (values.some((value, id) => value !== this.current.values[id]) || this.current.pending !== this.pending.size || error !== this.current.error) this.publish(values, error);
  }
  receive = (frame: Frame) => {
    if (frame.sequence !== undefined) {
      if (frame.sequence <= this.lastSequence || frame.revision! < this.lastRevision || frame.monotonicNs! < this.lastMonotonicNs || frame.frames < this.lastFrame) return false;
      this.lastSequence = frame.sequence; this.lastRevision = frame.revision!; this.lastMonotonicNs = frame.monotonicNs!;
    } else if (frame.frames <= this.lastFrame) return false;
    this.lastFrame = frame.frames; this.latest = frame; this.authoritative = [...frame.values];
    this.reconcile();
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
    this.epoch++; this.pending.clear(); this.gesture = null; this.history = []; this.future = [];
    this.lastFrame = -1; this.lastSequence = -1; this.lastRevision = -1; this.lastMonotonicNs = -1; this.latest = null;
    this.publish([...this.authoritative], '');
  };
  acceptPresetBarrier = (commandId: number, submission?: Submission, expectedValues?: number[]) => {
    for (const [id, pending] of this.pending) if (pending.commandId < commandId) this.pending.delete(id);
    this.gesture = null; this.history = []; this.future = [];
    if (this.transport.supportsAck && submission?.acceptedCommandId && expectedValues) {
      expectedValues.forEach((value, id) => {
        if (!this.pending.has(id)) this.pending.set(id, { value, commandId, accepted: true, acceptedCommandId: submission.acceptedCommandId, since: this.now() });
      });
    }
    this.publish(this.authoritative.map((value, id) => this.pending.get(id)?.value ?? value));
    this.reconcile();
  };
  undo = () => {
    this.end(); const edits = this.history.pop(); if (!edits) return;
    this.future.push(edits); for (const edit of edits) this.submit(edit.id, edit.before); this.publish();
  };
  redo = () => {
    this.end(); const edits = this.future.pop(); if (!edits) return;
    this.history.push(edits); for (const edit of edits) this.submit(edit.id, edit.after); this.publish();
  };
  applySnapshot = (values: number[]) => {
    if (values.length !== specs.length || values.some((v, i) => !Number.isFinite(v) || normalize(specs[i], v) !== v)) throw Error('Invalid preset values');
    this.end(); this.begin(0);
    for (let id = 0; id < values.length; id++) if (values[id] !== this.current.values[id]) this.edit(id, values[id]);
    this.end();
  };
}
