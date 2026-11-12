# Circuit-to-DSP traceability (#4)

No row is SPICE_VERIFIED or DEVICE_VERIFIED. Primary-reference access limitations
and digital oracle details are in [PHYSICAL_MODEL_AUDIT.md](PHYSICAL_MODEL_AUDIT.md).
“SOURCE_INSPECTED” refers to the actual local implementation; it never implies
that a cited but unavailable datasheet was read firsthand.

| Block | Current implementation / provenance | Required physical mapping | Status |
| --- | --- | --- | --- |
| CEM3320 gain cells | Four equal TPT poles, input/global odd rational limiter; SOURCE_INSPECTED | Exact Rev3 revision, IC gain-cell current law, capacitor/resistor values, CV/bias/temperature | OPEN / NOT_TESTED |
| CEM3320 feedback | Chosen 5 Hz HP, k=4.5*r, 6-iteration bounded Newton; SOURCE_INSPECTED | AC-coupling RC and resonance CV law from netlist; H2/H3 vs level/fc | OPEN / NOT_TESTED |
| CEM3320 discretization | Bilinear small-signal analytic oracle plus residual/stress tests; HOST_VERIFIED | Independent continuous KCL/SPICE model sweep before physical promotion | Digital tests PROVEN; physical OPEN |
| Moog differential stages | Normalized phi difference ODE, nested Newton; SOURCE_INSPECTED | V_T=k_B*T/q, matched pair scale, I_ctl/C, bias point, actual tanh vs rational error | OPEN / NOT_TESTED |
| Moog bass compensation | 1+k*bass*.65, dimensionless artistic choice; SOURCE_INSPECTED | Measured gain sag, desired compensation law, operating point | OPEN / NOT_TESTED |
| Curtis↔Moog | Linear output blend; inactive branch reset on reentry; HOST_VERIFIED selected transitions | Phase/level comparison by frequency, rapid modulation/high-Q transitions | PROVEN selected digital cases; OPEN full qualification |
| CEM3310 | 1.2 target / 1 threshold, ln6 attack, 4.8 decay/release constant; SOURCE_INSPECTED | Datasheet voltages, external RC/CV, GATE/TRIGGER/retrigger, tolerances | Time telemetry PROVEN; circuit OPEN |
| CEM3340 | Phase accumulator, PWM, PolyBLEP/BLAMP, sub-sample phase-reset sync; SOURCE_INSPECTED | Triangle slope reversal, selected edge/polarity, external sync RC, V/oct/PWM law | Existing digital tests PROVEN; circuit OPEN |
| MN3009/MN3101 | Hermite delay and triangular LFO, heuristic bias/saturation/noise; SOURCE_INSPECTED | Two-phase clock, 256 stages, transfer efficiency/leakage, bias, switching and analog filters | Musical mode retained; physical OPEN |
| Reverb | Eight-line normalized Hadamard FDN, ensemble, granular shimmer, vowel/damping; HOST_VERIFIED functional subset | Behavioral acoustic metrics, not electrical/proprietary reverse equivalence | Bypass PROVEN; RT60 calibration FAILED; other acoustic metrics OPEN |

For an assumed linear RC cell, KCL yields `C dv/dt=g_m(u-v)` and
`H(s)=1/(1+s*C/g_m)`; four equal cascades with no resonance give the fourth
power. This is a useful independent mathematical oracle, **not** enough to identify
the CEM3320's actual current-input/output cell. Physical derivation must specify
units, capacitor voltages, current sign, bias/operating point and temperature
before applying TPT/prewarp. Check transistor/cell nonlinear laws before their
discretization; never fit arbitrary “drive” to imply thermal calibration.

For an ideal N-stage two-phase BBD, when f_clk denotes a **complete two-phase
cycle**, two stage transfers occur per cycle and `delay=N/(2*f_clk)`. If N=256
and delay=3 ms, this convention requires f_clk≈42.67 kHz. Verify the actual driver
frequency convention, aperture and sampling bandwidth from the service manual
before comparing clocks. This kinematic delay identity does not model switched
capacitors. Required state equations include charge transfer/leakage and analog
anti-alias/reconstruction networks, not only a moving read pointer.

Promotion protocol: primary source/netlist → DC operating point → continuous
ODE/DAE and dimensioned parameters → independent SPICE/analytic reference →
discrete residual/stability/convergence → AC/phase, gain/Q, H2/H3/THD/IMD and
level/temperature sweeps → computational/latency envelope → controlled listening.
Until then preserve the measured VA baseline and classify proposed circuitry as
INFERRED or OPEN, never as physically verified.
