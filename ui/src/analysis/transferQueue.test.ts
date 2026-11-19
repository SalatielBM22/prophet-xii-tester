import { afterEach, describe, expect, it, vi } from 'vitest';
import { TransferQueue } from './transferQueue';
afterEach(() => vi.useRealTimers());
describe('bounded transfer measurements', () => {
  it('waits for active backend work before submitting only the newest pending intent', async () => {
    vi.useFakeTimers();
    const complete: ((value: number) => void)[] = [];
    const request = vi.fn(() => new Promise<number>(resolve => complete.push(resolve)));
    const publish = vi.fn(), queue = new TransferQueue(request, publish, vi.fn());
    queue.schedule([1]); await vi.advanceTimersByTimeAsync(180);
    queue.schedule([2]); queue.schedule([3]); await vi.advanceTimersByTimeAsync(180);
    expect(request).toHaveBeenCalledTimes(1);
    complete[0](1); await vi.advanceTimersByTimeAsync(0);
    expect(publish).not.toHaveBeenCalled();
    expect(request).toHaveBeenCalledTimes(2);
    expect(request.mock.calls[1]).toEqual([[3], expect.any(AbortSignal)]);
    complete[1](3); await vi.advanceTimersByTimeAsync(0);
    expect(publish).toHaveBeenCalledWith(3); queue.dispose();
  });
});
describe('continuous parameter gestures', () => {
  it('keeps measuring while values change every frame instead of waiting for the gesture to stop', async () => {
    vi.useFakeTimers();
    const request = vi.fn(async (values: number[]) => values[0]);
    const publish = vi.fn(), queue = new TransferQueue(request, publish, vi.fn());
    for (let i = 1; i <= 60; i++) { queue.schedule([i]); await vi.advanceTimersByTimeAsync(16); }
    // About one second of a 60 Hz drag: a trailing debounce issues zero requests here.
    expect(request.mock.calls.length).toBeGreaterThanOrEqual(4);
    expect(publish).toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(200);
    expect(publish).toHaveBeenLastCalledWith(60);
    queue.dispose();
  });
});
