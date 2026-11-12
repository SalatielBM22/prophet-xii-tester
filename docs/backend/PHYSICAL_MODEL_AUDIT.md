# Physical-model audit (#4)

Checkpoint, not a claim of circuit fidelity. No replacement physical model was
promoted. Current mathematical VA models and their solvers are retained.
Source baseline `b47f2d6`; changes on `research/physical-models-issue-4` descend
from the local RT-fix branch. Original artifacts remain intact.

## Evidence and authority

PROVEN / SOURCE_INSPECTED: local DSP source and upstream issues #1–#4 were read.
The issue's datasheet assertions are secondary evidence, not new measurements.
Attempts to retrieve CEM3320/CEM3310 original datasheets, Huovilainen DAFx paper
and Juno service manual failed with proxy 403; exact URLs/results are retained
in [primary-source-access.json](evidence/primary-source-access.json).
No schematic, transistor SPICE model, component tolerance table, operating point
or physical reference recording was independently supplied/verified here.

OPEN / NOT_TESTED: original CEM3320 datasheet + exact Prophet Rev3 revision/netlist,
component values and bias; Alexander & Sadiku RC/KCL/KVL/Laplace derivation against
that circuit; Huovilainen 2004 and D'Angelo/Välimäki 2013/2014 equations mapped to
this code; original CEM3340/CEM3310 and MN3009/MN3101/Juno circuit comparison.
References identify the next authority to inspect; they do not certify this code.
There is no SPICE_VERIFIED, ANDROID_EMULATOR_VERIFIED or DEVICE_VERIFIED result.

## CEM3320 priority: what can be derived now

INFERRED / SOURCE_INSPECTED: an **idealized assumed** gain-cell cascade can be
written `C_i dv_i/dt = g_m,i*(v_(i-1)-v_i)`, with `v_0 = x-k*f(v_4)`.
KCL supplies capacitor voltages as states, not the label “Curtis”. This is an
abstract RC/transconductance model; actual CEM gain-cell current relations,
external RC topology, bias, CV laws and asymmetry are missing. Therefore it is
not a derived Prophet Rev3 circuit. No capacitor or transconductance value is
invented to make a fit appear physical.

PROVEN / SOURCE_INSPECTED: actual implementation uses equal one-pole TPT stages:
`g=tan(pi*fc/fs)`, `G=g/(1+g)`, `y_i=G*x_i+(1-G)*s_i`, `s'_i=2*y_i-s_i`.
The fourth stage is `y4=G^4*u+S` with
`S=(1-G)*(G^3*s0+G^2*s1+G*s2+s3)`.
The input and global feedback use an **odd clamped rational function**
`phi(x)=x*(27+x²)/(27+9*x²)`, for |x|<=3, saturated outside. It is not exact tanh.
Feedback HP is `(1-Gdc)*(y4-sdc)` with a chosen 5 Hz corner, `k=4.5*resonance`.
Neither 5 Hz nor 4.5 has a verified schematic/calibration origin here.

For fixed states and admissible finite parameters, the scalar residual is
`F(y)=y-G^4*(phi(d*x)/d-k*phi(d*(1-Gdc)*(y-sdc))/d)-S`.
Its derivative is positive: `1+G^4*k*(1-Gdc)*phi'(...)`, since the rational
derivative is nonnegative. This supports uniqueness of this scalar root; it
does not prove every bounded Newton iteration converges to a hardware solution.
The code's maximum remains six iterations and the existing residual tests remain.

PROVEN / HOST_VERIFIED: the independent analytic digital oracle tests at
48/96 kHz, fc=80/2000/18000 Hz and three coherent test frequencies pass for both
filter types at zero resonance/small signal. The oracle is
`H(z)=[G*(1+z^-1)/(1-(1-2G)*z^-1)]^4`, equivalently magnitude
`(1+(tan(w/2)/g)^2)^-2`, phase `-4*atan(tan(w/2)/g)`.
Tolerance is `2e-5 + .005*|H|` per complex component, accounting for f32 and
absolute Newton tolerance at 0.001 input, **not a hardware tolerance band**.
Existing nonlinear residual tests use 2e-5; stress tests sweep cutoff/resonance.
See [model-after.txt](evidence/model-after.txt).

OPEN / NOT_TESTED: CEM3320 H2/H3 against original IC, gain-vs-cutoff, resonance
CV law, auto-oscillation onset/frequency and level/temperature dependence.
An odd nonlinear path with zero-state symmetric excitation is not evidence of
the predominantly H2 behavior described by the issue's datasheet summary.
The DC blocker removes DC; it does not introduce gain-cell asymmetry.

## Moog ladder and morph

PROVEN / SOURCE_INSPECTED: normalized per-stage equation is
`y_i-s_i = g*(phi(d*x_i)-phi(d*y_i))/d`, with state update `s'_i=2*y_i-s_i`.
Global input is `x*(1+k*bass*.65)-k*y4`, `k=4.35*resonance`. Up to eight outer
Newton iterations each evaluate four stage roots of up to eight iterations;
a final evaluation updates states. These counts and equations were **not**
reduced. `drive` is dimensionless; no measured V_T, I_ctl, capacitance, DC
operating point or BJT matching connects it to a specific Moog circuit.

INFERRED: BJT differential-pair currents can motivate tanh-like normalized ODEs,
but the chosen rational saturation and compensation require independent circuit
comparison. A continuous ODE reference/SPICE sweep must precede changing these
laws or calling the result Huovilainen/D'Angelo-equivalent. OPEN / NOT_TESTED.

PROVEN / HOST_VERIFIED: a new test first failed because reentering an inactive
filter exposed its old state, then passed after resetting that branch on reentry.
Endpoint CPU behavior is retained; both filters are not run continuously. The
existing kernel 5 ms smoother blends the newly warming branch. The bounded DC
ramp test covers both directions; full audio-rate/high-resonance transition
qualification remains OPEN. Direct low-level instantaneous switches are not
claimed click-free. Blend remains `(1-m)*Curtis + m*Moog`; no equal-power,
phase-coherence or level-preservation claim survives merely from shared TPT.

## CEM3310

PROVEN / SOURCE_INSPECTED: normalized attack target 1.2, threshold 1, coefficient
`1-exp(-ln(6)/samples)`; decay/release target undershoot .0015, coefficient
`1-exp(-4.8/samples)`. Attack preserves starting voltage on retrigger unless reset.
Software note gate is not independent electrical GATE/TRIGGER circuitry.
48/96 kHz, attack 1 ms/1 s, S=0/.5/1 telemetry/time tests pass.

INFERRED: if the issue's reported 6.5 V/5 V datasheet values are confirmed, their
normalized asymptote 1.3 implies `ln(1.3/.3)`, not ln(6). Renormalizing coefficients
could keep time-to-threshold equal while changing the curve. Therefore “attack
duration is wrong by 22%” would be unjustified. Values are not changed without
the primary circuit authority. Physical timing/temperature/GATE/TRIGGER remain OPEN.

## CEM3340 and MN3009

PROVEN / SOURCE_INSPECTED: oscillator uses band-limited saw/pulse and triangle
correction, audio-rate PWM and B-wrap-triggered phase reset of A with sub-sample
correction. This is phase-reset hard sync, not a verified IC triangle-slope reversal
under a known pulse polarity. Existing alias rejection/sync tests are digital only.
OPEN: datasheet/schematic PWM law, sync edge, voltage scaling and tolerance.

PROVEN / SOURCE_INSPECTED: chorus is fractional digital delay with Hermite
interpolation, pre/post TPT low-pass, heuristic asymmetric saturation, emphasis,
de-emphasis and optional noise. It is an efficient musical **BBD-style mode**,
not a simulated 256-capacitor charge-transfer line. No MN3101 clock waveform,
charge leakage, kT/C, switch resistance/aperture, bias calibration or clock
feedthrough is implemented. Modes I/II/I+II have finite/stereo regression tests.
No high-fidelity circuit mode is offered without calibration and CPU evidence.

## Digital reverb

An independent FDN/ensemble/shimmer interpretation, with no proprietary
Solaris/Cloudburst equivalence claim. The normalized Hadamard is orthogonal;
per-line nominal decay gain is `10^(-3*delay_sec/decay_sec)`. This law applies to
an ideal lossless loop, **not** the complete damped/vowel/shimmer system.

PROVEN / HOST_VERIFIED: impulse tests at 48/96 kHz produced
[reverb-rt60.csv](evidence/reverb-rt60.csv). T20 is an extrapolation from Schroeder
energy slope -5..-25 dB, using a finite `4*Decay+1` second record; it is not a
full 60 dB direct observation. At 1.5 s requested: neutral ~1.24–1.25 s, artistic
defaults ~.45 s. FAILED: user-facing calibrated RT60 acceptance. Per-band RT60,
late density, shimmer pitch accuracy, parameter-by-parameter musical sensitivity
and freeze energy drift qualification are OPEN. No feedback gain is blindly
boosted to hide losses, which could destabilize other frequencies.

Bypass/tail, zero-wet progression, freeze input isolation, reset determinism,
affine wet/dry law and finite ensemble/shimmer tests pass. These functional fixes
do not close the remaining reverb calibration gate. Size was not relabeled Decay.
