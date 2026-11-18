// SPDX-License-Identifier: GPL-3.0-only
import React, { memo } from 'react';
import type { FilterPoint, FilterResponseReport, FilterValidity } from '../protocol/enginePort';
import { filterPath, type FilterResponse } from '../analysis/spectrum';

interface Props {
  report: FilterResponseReport | null;
  legacyFilter?: FilterResponse | null;
  status: string;
  cutoffHz: number;
  resonance: number;
  morph: number;
  connected: boolean;
  onRefresh?: () => void;
}

function buildReportPath(points: FilterPoint[], phase: boolean, loHz: number, hiHz: number): string {
  if (points.length < 2 || hiHz <= loHz) return '';
  const logSpan = Math.log(hiHz / loHz);
  let drawing = false;
  return points
    .map(p => {
      const value = phase ? p.phase : p.db;
      if (value === null || !Number.isFinite(value)) {
        drawing = false;
        return '';
      }
      const x = (500 * Math.log(Math.max(loHz, p.hz) / loHz)) / logSpan;
      const y = phase ? 78 - value * 12 : Math.max(4, Math.min(144, 38 - value * 1.15));
      const cmd = drawing ? 'L' : 'M';
      drawing = true;
      return `${cmd}${x.toFixed(2)},${y.toFixed(2)}`;
    })
    .join(' ');
}

function validityBadge(v: FilterValidity): { label: string; tone: 'ok' | 'warn' | 'err' } {
  switch (v) {
    case 'Measurable':
      return { label: 'MEASURABLE LINEAR', tone: 'ok' };
    case 'SelfOscillating':
      return { label: 'SELF-OSCILLATING', tone: 'warn' };
    case 'LongTail':
      return { label: 'LONG TAIL', tone: 'warn' };
    case 'NonlinearOperatingPoint':
      return { label: 'NONLINEAR OPERATING POINT', tone: 'warn' };
    case 'NumericalFailure':
      return { label: 'NUMERICAL FAILURE', tone: 'err' };
  }
}

export const VcfResponsePlot = memo(function VcfResponsePlot({
  report,
  legacyFilter,
  status,
  cutoffHz,
  resonance,
  morph,
  connected,
  onRefresh,
}: Props) {
  const isNative = report !== null;
  const isMeasurable = report ? report.validity === 'Measurable' && report.points.length >= 2 : Boolean(legacyFilter);
  const loHz = report?.points[0]?.hz ?? legacyFilter?.hz[0] ?? 20;
  const hiHz =
    report?.points[report.points.length - 1]?.hz ??
    legacyFilter?.hz[legacyFilter.hz.length - 1] ??
    20000;

  const magPath = report
    ? report.validity === 'Measurable'
      ? buildReportPath(report.points, false, loHz, hiHz)
      : ''
    : legacyFilter
      ? filterPath(legacyFilter, false)
      : '';

  const phasePath = report
    ? report.validity === 'Measurable'
      ? buildReportPath(report.points, true, loHz, hiHz)
      : ''
    : legacyFilter
      ? filterPath(legacyFilter, true)
      : '';

  const badge = report ? validityBadge(report.validity) : null;
  const cutoffX =
    hiHz > loHz && cutoffHz >= loHz
      ? Math.max(2, Math.min(498, (500 * Math.log(Math.min(hiHz, cutoffHz) / loHz)) / Math.log(hiHz / loHz)))
      : 0;

  return (
    <div className="vcf-plot" data-testid="vcf-response-plot">
      <div className="vcf-plot-head">
        <div>
          <span className="vcf-plot-title">VCF SMALL-SIGNAL RESPONSE</span>
          <small className="vcf-morph-tag">
            {morph < 0.25 ? 'CURTIS CEM3320' : morph > 0.75 ? 'MOOG LADDER' : `MORPH ${(morph * 100).toFixed(0)}%`} · Q {(resonance * 100).toFixed(0)}%
          </small>
        </div>
        <div className="vcf-plot-actions">
          {badge && <span className={`validity-pill tone-${badge.tone}`}>{badge.label}</span>}
          {onRefresh && (
            <button type="button" className="mini-btn" disabled={!connected} onClick={onRefresh}>
              MEASURE
            </button>
          )}
        </div>
      </div>

      {isNative && report.validity !== 'Measurable' ? (
        <div className="vcf-invalid-fallback" role="status" data-testid="vcf-invalid-fallback">
          <strong>Small-signal transfer curve withheld ({report.validity})</strong>
          <p>{report.reason}</p>
          <div className="vcf-metrics">
            <span>SR: {report.sampleRate} Hz</span>
            <span>Window: {report.windowSamples} smp</span>
            {report.tailRms !== null && <span>Tail RMS: {report.tailRms.toExponential(2)}</span>}
            {report.relativeScalingError !== null && (
              <span>Scaling err: {(report.relativeScalingError * 100).toFixed(2)}%</span>
            )}
          </div>
          <small>High-Q / self-oscillating regimes are nonlinear and cannot be represented by a linear Bode curve.</small>
        </div>
      ) : (
        <svg viewBox="0 0 500 150" role="img" aria-label="Adjacent VCF magnitude and phase transfer response">
          {[38, 61, 84, 107, 130].map(y => (
            <line key={y} x1="0" y1={y} x2="500" y2={y} stroke="#233028" strokeWidth="1" />
          ))}
          {cutoffX > 0 && (
            <line x1={cutoffX} y1="4" x2={cutoffX} y2="146" stroke="#d6ad6566" strokeDasharray="3 3" strokeWidth="1" />
          )}
          {magPath && <path d={magPath} fill="none" stroke="#e9b95e" strokeWidth="2.2" />}
          {phasePath && <path d={phasePath} fill="none" stroke="#6ec9a8" strokeWidth="1.2" opacity="0.85" />}
          <text x="6" y="14" className="svg-label">
            0 dB
          </text>
          <text x="6" y="144" className="svg-label">
            20 Hz
          </text>
          <text x="494" y="144" textAnchor="end" className="svg-label">
            {hiHz >= 1000 ? `${(hiHz / 1000).toFixed(1)} kHz` : `${Math.round(hiHz)} Hz`}
          </text>
        </svg>
      )}

      <div className="vcf-plot-foot" role="status">
        <span>
          {report
            ? `${report.sampleRate} Hz · ${report.windowSamples} smp · ${status}`
            : legacyFilter
              ? `${legacyFilter.sampleRate} Hz · ${status}`
              : status}
        </span>
        {isMeasurable && <small>Amber |H(f)| dB · Teal unwrapped phase</small>}
      </div>
    </div>
  );
});
