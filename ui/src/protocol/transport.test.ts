import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ControlTransport } from './transport';
describe('bounded ordered transport', () => {
  beforeEach(() => vi.useFakeTimers()); afterEach(() => vi.useRealTimers());
  it('coalesces A/B/A in monotonic command order, without crossing a preset barrier', async () => {
    let release!: (response: Response) => void;
    const seen: Array<{ commandId: number; type: string; id?: number; value?: number }> = [];
    const request: typeof fetch = vi.fn((_url, init) => {
      seen.push(JSON.parse(String(init?.body)));
      return seen.length === 1 ? new Promise<Response>(resolve => { release = resolve; }) : Promise.resolve(new Response(null, { status: 202 }));
    });
    const transport = new ControlTransport('', request);
    transport.metadata = { protocolVersion: 2, capabilities: ['parameterAck'], presets: [] };
    const active = transport.send({ type: 'panic' });
    const a1 = transport.send({ type: 'param', id: 1, value: 10 });
    const b = transport.send({ type: 'param', id: 2, value: .5 });
    const a2 = transport.send({ type: 'param', id: 1, value: 20 });
    await vi.advanceTimersByTimeAsync(16);
    const preset = transport.send({ type: 'preset', index: 0 });
    const laterB = transport.send({ type: 'param', id: 2, value: .7 });
    await vi.advanceTimersByTimeAsync(16);
    release(new Response(null, { status: 202 }));
    await Promise.all([active, a1, b, a2, preset, laterB]);
    expect(seen.map(x => x.commandId)).toEqual([1, 3, 4, 5, 6]);
    expect(seen.map(x => x.type)).toEqual(['panic', 'param', 'param', 'preset', 'param']);
    expect(seen[1].value).toBe(.5); expect(seen[4].value).toBe(.7); transport.dispose();
  });
  it('reset clears timer state so reconnect parameters submit normally', async () => {
    const request = vi.fn(() => Promise.resolve(new Response(null, { status: 202 }))), transport = new ControlTransport('', request);
    const first = transport.send({ type: 'param', id: 1, value: 0 }).catch(() => false);
    transport.reset(); await first;
    const second = transport.send({ type: 'param', id: 1, value: 1 });
    await vi.advanceTimersByTimeAsync(16); await second;
    expect(request).toHaveBeenCalledTimes(1); transport.dispose();
  });
  it('drops superseded parameter values without unbounded work', async () => {
    const request = vi.fn(() => Promise.resolve(new Response(null, { status: 202 }))), transport = new ControlTransport('', request);
    const promises = Array.from({ length: 1000 }, (_, i) => transport.send({ type: 'param', id: 23, value: i }));
    await vi.advanceTimersByTimeAsync(16); const results = await Promise.all(promises);
    expect(results.filter(Boolean)).toHaveLength(1); expect(request).toHaveBeenCalledTimes(1); transport.dispose();
  });
});
