/** At most one HTTP measurement in flight plus one replaceable pending intent. */
export class TransferQueue<T> {
  private generation = 0;
  private latest: { values: number[]; generation: number } | null = null;
  private active = false;
  private timer: ReturnType<typeof setTimeout> | undefined;
  private disposed = false;
  private controller = new AbortController();
  constructor(private request: (values: number[], signal: AbortSignal) => Promise<T>, private publish: (response: T) => void, private error: (error: unknown) => void) {}
  schedule(values: readonly number[]) {
    if (this.disposed) return;
    this.latest = { values: [...values], generation: ++this.generation };
    clearTimeout(this.timer);
    this.timer = setTimeout(() => { this.timer = undefined; void this.drain(); }, 180);
  }
  private async drain() {
    if (this.active || this.timer || this.disposed || !this.latest) return;
    const intent = this.latest; this.latest = null; this.active = true;
    try {
      const response = await this.request(intent.values, this.controller.signal);
      if (!this.disposed && intent.generation === this.generation) this.publish(response);
    } catch (error) { if (!this.disposed && intent.generation === this.generation) this.error(error); }
    finally { this.active = false; if (!this.disposed) void this.drain(); }
  }
  dispose() { this.disposed = true; clearTimeout(this.timer); this.latest = null; this.controller.abort(); }
}
