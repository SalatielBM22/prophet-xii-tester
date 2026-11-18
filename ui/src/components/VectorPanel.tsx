// SPDX-License-Identifier: GPL-3.0-only
import React, { memo, useRef, useState } from 'react';
import {
  ROUTES,
  TOPOLOGIES,
  clampToDiamond,
  computeVectorWeights,
  type UiPatchState,
  type UiPoint,
  type UiRoute,
  type UiStage,
  type UiTopology,
} from '../protocol/enginePort';

interface Props {
  patch: UiPatchState;
  disabled: boolean;
  supported: boolean;
  onVectorXY: (x: number, y: number) => void;
  onUpdatePatch: (next: {
    topology: UiTopology;
    routes: [UiRoute, UiRoute];
    ratios: [number, number];
    stages: [UiStage, UiStage, UiStage, UiStage, UiStage];
    modulation: [UiPoint, UiPoint, UiPoint];
    loopHeld: boolean;
  }) => void;
}

const STAGE_LABELS = ['S1 Attack', 'S2 Decay', 'S3 Mid', 'S4 Sustain', 'S5 Release'] as const;
const MOD_LABELS = ['Velocity', 'Pressure', 'Bipolar LFO'] as const;
const CORNER_LABELS = ['A', 'B', 'C', 'D'] as const;

export const VectorPanel = memo(function VectorPanel({
  patch,
  disabled,
  supported,
  onVectorXY,
  onUpdatePatch,
}: Props) {
  const padRef = useRef<SVGSVGElement>(null);
  const dragging = useRef(false);
  const [localXY, setLocalXY] = useState<UiPoint | null>(null);
  const [selectedStage, setSelectedStage] = useState<number>(0);

  const displayPos = localXY ?? patch.position;
  const displayWeights = localXY ? computeVectorWeights(localXY.x, localXY.y) : patch.weights;
  const isVector = patch.topology === 'Vector4';
  const hasOscC = patch.topology !== 'Classic2';
  const hasOscD = patch.topology === 'Extended4' || patch.topology === 'Vector4';
  const topologyLocked = !patch.idleForTopologyChange;

  const commitPatch = (
    overrides: Partial<{
      topology: UiTopology;
      routes: [UiRoute, UiRoute];
      ratios: [number, number];
      stages: [UiStage, UiStage, UiStage, UiStage, UiStage];
      modulation: [UiPoint, UiPoint, UiPoint];
      loopHeld: boolean;
    }>,
  ) => {
    onUpdatePatch({
      topology: overrides.topology ?? patch.topology,
      routes: overrides.routes ?? patch.routes,
      ratios: overrides.ratios ?? patch.ratios,
      stages: overrides.stages ?? patch.stages,
      modulation: overrides.modulation ?? patch.modulation,
      loopHeld: overrides.loopHeld ?? patch.loopHeld,
    });
  };

  const clientToDiamond = (clientX: number, clientY: number): UiPoint => {
    const svg = padRef.current;
    if (!svg) return { x: 0, y: 0 };
    const rect = svg.getBoundingClientRect();
    const nx = ((clientX - rect.left) / Math.max(1, rect.width)) * 2 - 1;
    const ny = 1 - ((clientY - rect.top) / Math.max(1, rect.height)) * 2;
    return clampToDiamond(nx / 0.85, ny / 0.85);
  };

  const pushXY = (pt: UiPoint) => {
    setLocalXY(pt);
    onVectorXY(pt.x, pt.y);
  };

  const stage = patch.stages[selectedStage] ?? { target: { x: 0, y: 0 }, millis: 100 };

  // Convert diamond coordinates (-1..1) into SVG coordinates (center 100,100, radius 82)
  const toSvg = (pt: UiPoint) => ({
    cx: 100 + pt.x * 82,
    cy: 100 - pt.y * 82,
  });

  const joySvg = toSvg(displayPos);
  const stagePoly = patch.stages
    .map(s => {
      const p = toSvg(s.target);
      return `${p.cx.toFixed(1)},${p.cy.toFixed(1)}`;
    })
    .join(' ');

  if (!supported) {
    return (
      <section className="panel vector-panel" aria-label="Oscillator Topology and Vector4">
        <div className="panel-head">
          <h2>OSCILLATOR TOPOLOGY &amp; VECTOR4</h2>
          <span className="pill muted">HOST LEGACY MODE</span>
        </div>
        <p className="muted-note">
          Extended3, Extended4, typed FM/Sync/Ring routing, and Vector4 diamond synthesis require an EnginePort v1 host
          (Standalone native window, VST3/CLAP, LV2, or Android APK).
        </p>
      </section>
    );
  }

  return (
    <section className="panel vector-panel wide" aria-label="Oscillator Topology and Vector4">
      <div className="panel-head">
        <h2>OSCILLATOR TOPOLOGY &amp; VECTOR4</h2>
        <div className="topology-pills" role="group" aria-label="Oscillator topology">
          {TOPOLOGIES.map(topo => {
            const active = patch.topology === topo;
            const locked = !active && topologyLocked;
            return (
              <button
                key={topo}
                type="button"
                className={`seg-btn ${active ? 'selected' : ''}`}
                disabled={disabled || locked}
                aria-pressed={active}
                title={
                  locked
                    ? 'Voices are sounding — release all keys so topology changes cleanly without dropping notes'
                    : `Switch oscillator topology to ${topo}`
                }
                onClick={() => {
                  if (!active) commitPatch({ topology: topo });
                }}
              >
                {topo}
              </button>
            );
          })}
        </div>
      </div>

      {topologyLocked && (
        <div className="inline-guard" role="status" data-testid="topology-idle-guard">
          Voices active: release held notes before switching oscillator topology (notes are never silently dropped).
        </div>
      )}

      <div className="vector-grid">
        {/* Left: Ratios & Typed Routing Matrix */}
        <div className="vector-routing">
          <h3>EXTENDED SOURCES &amp; ROUTING</h3>
          <div className="route-row">
            <label className={!hasOscC ? 'dimmed' : ''}>
              <span>Osc C Ratio</span>
              <input
                type="number"
                aria-label="Osc C Ratio"
                min={0.0625}
                max={16}
                step={0.0625}
                disabled={disabled || !hasOscC}
                value={Number(patch.ratios[0].toFixed(4))}
                onChange={e => {
                  const v = Number(e.target.value);
                  if (Number.isFinite(v) && v >= 0.0625 && v <= 16) {
                    commitPatch({ ratios: [v, patch.ratios[1]] });
                  }
                }}
              />
            </label>
            <label className={!hasOscC ? 'dimmed' : ''}>
              <span>B → C Route</span>
              <select
                aria-label="Osc B to C Route"
                disabled={disabled || !hasOscC}
                value={patch.routes[0]}
                onChange={e => commitPatch({ routes: [e.target.value as UiRoute, patch.routes[1]] })}
              >
                {ROUTES.map(r => (
                  <option key={r} value={r}>
                    {r}
                  </option>
                ))}
              </select>
            </label>
          </div>

          <div className="route-row">
            <label className={!hasOscD ? 'dimmed' : ''}>
              <span>Osc D Ratio</span>
              <input
                type="number"
                aria-label="Osc D Ratio"
                min={0.0625}
                max={16}
                step={0.0625}
                disabled={disabled || !hasOscD}
                value={Number(patch.ratios[1].toFixed(4))}
                onChange={e => {
                  const v = Number(e.target.value);
                  if (Number.isFinite(v) && v >= 0.0625 && v <= 16) {
                    commitPatch({ ratios: [patch.ratios[0], v] });
                  }
                }}
              />
            </label>
            <label className={!hasOscD ? 'dimmed' : ''}>
              <span>C → D Route</span>
              <select
                aria-label="Osc C to D Route"
                disabled={disabled || !hasOscD}
                value={patch.routes[1]}
                onChange={e => commitPatch({ routes: [patch.routes[0], e.target.value as UiRoute] })}
              >
                {ROUTES.map(r => (
                  <option key={r} value={r}>
                    {r}
                  </option>
                ))}
              </select>
            </label>
          </div>

          <div className="vector-weights" aria-label="Vector source weights">
            {CORNER_LABELS.map((lbl, idx) => (
              <div key={lbl} className={`weight-bar ${!isVector && idx >= 2 ? 'dimmed' : ''}`}>
                <div className="weight-head">
                  <strong>{lbl}</strong>
                  <span>{Math.round((displayWeights[idx] ?? 0) * 100)}%</span>
                </div>
                <div className="weight-track">
                  <div className="weight-fill" style={{ width: `${Math.min(100, Math.round((displayWeights[idx] ?? 0) * 100))}%` }} />
                </div>
              </div>
            ))}
          </div>

          <small className="vector-caption">
            Diamond mix |x| + |y| ≤ 1 blends sources A/B/C/D (not stereo pan). Engine applies 5 ms joystick smoothing.
          </small>
        </div>

        {/* Center: Interactive Diamond XY Joystick */}
        <div className="vector-joystick-wrap">
          <div className="joystick-head">
            <span>XY VECTOR MIXER</span>
            <code>
              X {displayPos.x.toFixed(2)} · Y {displayPos.y.toFixed(2)}
            </code>
          </div>
          <svg
            ref={padRef}
            className={`vector-diamond ${!isVector ? 'inactive' : ''}`}
            viewBox="0 0 200 200"
            role="application"
            tabIndex={disabled || !isVector ? -1 : 0}
            aria-label="Vector4 XY diamond joystick"
            onPointerDown={e => {
              if (disabled || !isVector || e.button !== 0) return;
              e.preventDefault();
              e.currentTarget.setPointerCapture(e.pointerId);
              dragging.current = true;
              pushXY(clientToDiamond(e.clientX, e.clientY));
            }}
            onPointerMove={e => {
              if (!dragging.current || disabled || !isVector) return;
              pushXY(clientToDiamond(e.clientX, e.clientY));
            }}
            onPointerUp={() => {
              dragging.current = false;
              setLocalXY(null);
            }}
            onPointerCancel={() => {
              dragging.current = false;
              setLocalXY(null);
            }}
            onDoubleClick={() => {
              if (!disabled && isVector) {
                setLocalXY(null);
                onVectorXY(0, 0);
              }
            }}
            onKeyDown={e => {
              if (disabled || !isVector) return;
              const step = e.shiftKey ? 0.02 : 0.08;
              let nx = displayPos.x;
              let ny = displayPos.y;
              if (e.key === 'ArrowLeft') nx -= step;
              else if (e.key === 'ArrowRight') nx += step;
              else if (e.key === 'ArrowUp') ny += step;
              else if (e.key === 'ArrowDown') ny -= step;
              else if (e.key === 'Home') {
                nx = 0;
                ny = 0;
              } else return;
              e.preventDefault();
              pushXY(clampToDiamond(nx, ny));
              setLocalXY(null);
            }}
          >
            {/* Diamond boundary: A(-1,0), B(0,1), C(1,0), D(0,-1) */}
            <polygon points="18,100 100,18 182,100 100,182" fill="#0d1310" stroke="#7c876c" strokeWidth="1.6" />
            <polygon points="59,100 100,59 141,100 100,141" fill="none" stroke="#2c3a30" strokeDasharray="3 3" />
            <line x1="18" y1="100" x2="182" y2="100" stroke="#2f3c32" strokeWidth="1" />
            <line x1="100" y1="18" x2="100" y2="182" stroke="#2f3c32" strokeWidth="1" />
            {/* 5-stage vector trajectory path */}
            <polyline points={stagePoly} fill="none" stroke="#6ec9a8" strokeWidth="1.4" strokeDasharray="4 2" opacity={isVector ? 0.85 : 0.3} />
            {patch.stages.map((s, idx) => {
              const p = toSvg(s.target);
              return (
                <circle
                  key={idx}
                  cx={p.cx}
                  cy={p.cy}
                  r={selectedStage === idx ? 4.5 : 3}
                  fill={selectedStage === idx ? '#6ec9a8' : '#3a6e5c'}
                  stroke="#0b100e"
                  strokeWidth="1"
                />
              );
            })}
            {/* Live joystick handle */}
            <circle cx={joySvg.cx} cy={joySvg.cy} r="7.5" fill={isVector ? '#e9b95e' : '#5f6359'} stroke="#121614" strokeWidth="2" />
            <text x="8" y="104" className="svg-corner">
              A
            </text>
            <text x="100" y="13" textAnchor="middle" className="svg-corner">
              B
            </text>
            <text x="192" y="104" textAnchor="end" className="svg-corner">
              C
            </text>
            <text x="100" y="196" textAnchor="middle" className="svg-corner">
              D
            </text>
          </svg>
        </div>

        {/* Right: 5-Stage Trajectory & Modulation Depths */}
        <div className="vector-trajectory">
          <div className="traj-head">
            <h3>5-STAGE TRAJECTORY</h3>
            <label className="inline-check">
              <input
                type="checkbox"
                disabled={disabled || !isVector}
                checked={patch.loopHeld}
                onChange={e => commitPatch({ loopHeld: e.target.checked })}
              />
              Loop Held
            </label>
          </div>
          <div className="stage-tabs" role="tablist" aria-label="Vector envelope stages">
            {STAGE_LABELS.map((lbl, i) => (
              <button
                key={lbl}
                type="button"
                role="tab"
                aria-selected={selectedStage === i}
                className={`mini-btn ${selectedStage === i ? 'selected' : ''}`}
                disabled={disabled || !isVector}
                onClick={() => setSelectedStage(i)}
              >
                {`S${i + 1}`}
              </button>
            ))}
          </div>
          <div className="stage-editor">
            <small>{STAGE_LABELS[selectedStage]}</small>
            <div className="stage-inputs">
              <label>
                X
                <input
                  type="number"
                  aria-label="Stage target X"
                  min={-1}
                  max={1}
                  step={0.05}
                  disabled={disabled || !isVector}
                  value={Number(stage.target.x.toFixed(2))}
                  onChange={e => {
                    const x = Number(e.target.value);
                    if (!Number.isFinite(x)) return;
                    const clamped = clampToDiamond(x, stage.target.y);
                    const nextStages = patch.stages.map((s, idx) =>
                      idx === selectedStage ? { ...s, target: clamped } : s,
                    ) as [UiStage, UiStage, UiStage, UiStage, UiStage];
                    commitPatch({ stages: nextStages });
                  }}
                />
              </label>
              <label>
                Y
                <input
                  type="number"
                  aria-label="Stage target Y"
                  min={-1}
                  max={1}
                  step={0.05}
                  disabled={disabled || !isVector}
                  value={Number(stage.target.y.toFixed(2))}
                  onChange={e => {
                    const y = Number(e.target.value);
                    if (!Number.isFinite(y)) return;
                    const clamped = clampToDiamond(stage.target.x, y);
                    const nextStages = patch.stages.map((s, idx) =>
                      idx === selectedStage ? { ...s, target: clamped } : s,
                    ) as [UiStage, UiStage, UiStage, UiStage, UiStage];
                    commitPatch({ stages: nextStages });
                  }}
                />
              </label>
              <label>
                ms
                <input
                  type="number"
                  aria-label="Stage duration ms"
                  min={1}
                  max={60000}
                  step={10}
                  disabled={disabled || !isVector}
                  value={stage.millis}
                  onChange={e => {
                    const ms = Math.round(Number(e.target.value));
                    if (!Number.isFinite(ms) || ms < 1 || ms > 65535) return;
                    const nextStages = patch.stages.map((s, idx) =>
                      idx === selectedStage ? { ...s, millis: ms } : s,
                    ) as [UiStage, UiStage, UiStage, UiStage, UiStage];
                    commitPatch({ stages: nextStages });
                  }}
                />
              </label>
            </div>
          </div>

          <h3 className="subhead">XY MODULATION (VEL / PRESS / LFO)</h3>
          <div className="mod-grid">
            {MOD_LABELS.map((lbl, idx) => {
              const m = patch.modulation[idx] ?? { x: 0, y: 0 };
              return (
                <div key={lbl} className="mod-item">
                  <span>{lbl}</span>
                  <input
                    type="number"
                    aria-label={`${lbl} X depth`}
                    min={-1}
                    max={1}
                    step={0.05}
                    disabled={disabled || !isVector}
                    value={Number(m.x.toFixed(2))}
                    onChange={e => {
                      const x = Number(e.target.value);
                      if (!Number.isFinite(x)) return;
                      const nextMod = patch.modulation.map((pt, i) =>
                        i === idx ? clampToDiamond(x, pt.y) : pt,
                      ) as [UiPoint, UiPoint, UiPoint];
                      commitPatch({ modulation: nextMod });
                    }}
                  />
                  <input
                    type="number"
                    aria-label={`${lbl} Y depth`}
                    min={-1}
                    max={1}
                    step={0.05}
                    disabled={disabled || !isVector}
                    value={Number(m.y.toFixed(2))}
                    onChange={e => {
                      const y = Number(e.target.value);
                      if (!Number.isFinite(y)) return;
                      const nextMod = patch.modulation.map((pt, i) =>
                        i === idx ? clampToDiamond(pt.x, y) : pt,
                      ) as [UiPoint, UiPoint, UiPoint];
                      commitPatch({ modulation: nextMod });
                    }}
                  />
                </div>
              );
            })}
          </div>
        </div>
      </div>
    </section>
  );
});
