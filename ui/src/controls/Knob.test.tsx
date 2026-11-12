import React from 'react';
import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';
import specs from '../param-specs.json';
import { Knob } from './Knob';
const cutoff = specs[23];
function mount(resetValue?: number) {
  const props = { spec: cutoff, value: 680, disabled: false, reason: '', resetValue, onChange: vi.fn(), onBegin: vi.fn(), onEnd: vi.fn(), onLearn: vi.fn() };
  render(<Knob {...props} />); return props;
}
describe('Knob accessibility and numeric semantics', () => {
  it('commits valid numeric values, reports invalid values and cancels Escape', () => {
    const props = mount(); fireEvent.click(screen.getByRole('button', { name: 'Edit Filter Cutoff' }));
    const input = screen.getByRole('textbox', { name: 'Filter Cutoff value' });
    fireEvent.change(input, { target: { value: '20000' } }); fireEvent.keyDown(input, { key: 'Enter' });
    expect(screen.getByRole('alert')).toHaveTextContent('20'); expect(props.onChange).not.toHaveBeenCalled();
    fireEvent.change(input, { target: { value: '1234.5' } }); fireEvent.keyDown(input, { key: 'Enter' });
    expect(props.onChange).toHaveBeenCalledWith(23, 1234.5);
    fireEvent.click(screen.getByRole('button', { name: 'Edit Filter Cutoff' }));
    fireEvent.change(screen.getByRole('textbox'), { target: { value: '500' } }); fireEvent.keyDown(screen.getByRole('textbox'), { key: 'Escape' });
    expect(props.onChange).toHaveBeenCalledTimes(1);
  });
  it('resets to the supplied immutable factory reference and participates in edit history', () => {
    const props = mount(1200); fireEvent.doubleClick(screen.getByRole('slider', { name: 'Filter Cutoff' }));
    expect(props.onChange).toHaveBeenCalledWith(23, 1200);
  });
  it('does not guess a reset value without a factory reference', () => {
    const props = mount(); fireEvent.doubleClick(screen.getByRole('slider', { name: 'Filter Cutoff' })); expect(props.onChange).not.toHaveBeenCalled();
  });
  it('groups held keyboard arrows until keyup and exposes contextual MIDI Learn', () => {
    const props = mount(), slider = screen.getByRole('slider', { name: 'Filter Cutoff' });
    fireEvent.keyDown(slider, { key: 'ArrowUp' }); fireEvent.keyDown(slider, { key: 'ArrowUp', repeat: true });
    expect(props.onEnd).not.toHaveBeenCalled(); fireEvent.keyUp(slider, { key: 'ArrowUp' }); expect(props.onEnd).toHaveBeenCalledTimes(1);
    fireEvent.contextMenu(slider); expect(props.onLearn).toHaveBeenCalledWith(23);
  });
});
