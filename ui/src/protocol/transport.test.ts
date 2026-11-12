import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ControlTransport } from './transport';
describe('bounded ordered transport', () => {
  beforeEach(() => vi.useFakeTimers()); afterEach(() => vi.useRealTimers());
  it('coalesces A/B/A in monotonic command order, without crossing a preset barrier', async () => {
    let release!: (response: Response) => void;
    const seen: Array<{ type: string; id?: number; value?: number }> = [];
    const request: typeof fetch = vi.fn((_url, init) => {
      seen.push(JSON.parse(String(init?.body)));
      return seen.length === 1 ? new Promise<Response>(resolve => { release = resolve; }) : Promise.resolve(new Response(null, { status: 202 }));
    });
    const transport = new ControlTransport('', request);
    transport.metadata = { protocolVersion: 1, capabilities: [], presets: [] };
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
    expect(seen.map(x => x.id)).toEqual([undefined, 2, 1, undefined, 2]);
    expect(seen.every(x => !('clientId' in x) && !('commandId' in x))).toBe(true);
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
    expect(results.filter(result => result.submitted)).toHaveLength(1); expect(request).toHaveBeenCalledTimes(1); transport.dispose();
  });
});

describe('native acceptance and command forms', () => {
  it('returns enqueue ID separately and serializes only Agent B command fields', async () => {
    const seen: unknown[] = [];
    const request: typeof fetch = vi.fn((_url, init) => { seen.push(JSON.parse(String(init?.body))); return Promise.resolve(new Response(JSON.stringify({ acceptedCommandId: 41 + seen.length }), { status: 202 })); });
    const transport = new ControlTransport('', request);
    transport.metadata = { protocolVersion: 1, capabilities: ['parameterAck', 'midiLearnLifecycle', 'reverbBypass'], presets: [] };
    expect(await transport.send({ type: 'cancel_learn' })).toEqual({ submitted: true, acceptedCommandId: 42 });
    await transport.send({ type: 'unlearn', id: 23 });
    await transport.send({ type: 'remap', cc: 16, id: 23 });
    await transport.send({ type: 'reverb_bypass', enabled: false, tail: 'kill' });
    expect(seen).toEqual([{ type: 'cancel_learn' }, { type: 'unlearn', id: 23 }, { type: 'remap', cc: 16, id: 23 }, { type: 'reverb_bypass', enabled: false, tail: 'kill' }]);
    transport.dispose();
  });
  it('rejects malformed native acceptance while retaining legacy empty202 compatibility', async () => {
    const transport = new ControlTransport('', () => Promise.resolve(new Response('', { status: 202 })));
    expect(await transport.send({ type: 'panic' })).toEqual({ submitted: true, acceptedCommandId: null });
    transport.metadata = { protocolVersion: 1, capabilities: ['parameterAck'], presets: [] };
    await expect(transport.send({ type: 'panic' })).rejects.toThrow('acceptedCommandId');
    transport.dispose();
  });
});
