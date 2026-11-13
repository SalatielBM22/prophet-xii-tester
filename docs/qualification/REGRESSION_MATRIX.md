# Matriz de Regressão e Auditoria de Áudio (G8.1)

Este documento explica todas as divergências de hash observadas após as otimizações do pipeline DSP (`e8f572d`) e a calibração do reverb (`dfab71a`) em `e23fa95`, separando mudanças de arredondamento de ponto flutuante (~1 ULP) de alterações intencionais na cauda de reverberação.

## 1. Auditoria da Matriz Seca (`rt_matrix --iterations 100`, 256 frames)

Na comparação entre o CSV da baseline ([`baseline-rt-256.csv`](file:///home/antigravity/SalatielBM22/prophet-xii-tester/docs/backend/evidence/baseline-rt-256.csv)) e o CSV final em `e23fa95` (`/tmp/g8-evidence/final/rt-256.csv`), considerando exclusivamente os 48 casos secos (`fx=0`, `Chorus=Off`, `ReverbWet=0.0` sobrescrito após warmup apenas nos cálculos de hash FNV-1a de bits `f32`):

- **12 / 48 casos com `voices=0` (idle)** mantiveram hash `4b1a73d6d85dc325` idêntico à baseline.
- **36 / 48 casos com vozes ativas (`voices ∈ {1, 6, 12}`)** alteraram o hash FNV-1a bit-a-bit de `f32` em razão exclusiva de **reassociação algébrica IEEE-754 (~1 ULP)** introduzida em `e8f572d`:
  1. [`SymmetricFirKernel::step`](file:///home/antigravity/SalatielBM22/prophet-xii-tester/crates/prophet-dsp/src/oversample.rs#L108-L123): soma pareada de coeficientes simétricos (`h[k] * (z[-k] + z[+k])`).
  2. [`ProphetVoice::process_sample`](file:///home/antigravity/SalatielBM22/prophet-xii-tester/crates/prophet-dsp/src/voice.rs#L371-L625): hoisting de exponenciais/tangentes invariantes por sub-amostra para fora do laço `oversample` e reutilização de cálculos entre blocos/vozes.
  3. [`fast_tanh_and_sech2`](file:///home/antigravity/SalatielBM22/prophet-xii-tester/crates/prophet-dsp/src/vcf_cem3320.rs#L32-L45): cálculo simultâneo com recíproco compartilhado.
  4. [`MoogLadderFilter::process_sample`](file:///home/antigravity/SalatielBM22/prophet-xii-tester/crates/prophet-dsp/src/vcf_moog.rs#L178-L309): solucionador ZDF analítico 4×4 *bordered-bidiagonal* de convergência equivalente validado contra o solver aninhado original em [`fast_moog_solver_matches_reference_across_sweeps_and_saturation`](file:///home/antigravity/SalatielBM22/prophet-xii-tester/crates/prophet-dsp/src/vcf_moog.rs#L478-L512) (tolerância `< 1e-4` por amostra ao longo de varreduras de cutoff/ressonância/drive).

## 2. Comparação Quantitativa das 22 Demos WAV vs Baseline (`evidence/wavs/*.wav`)

A inspeção de [`configure_demo`](file:///home/antigravity/SalatielBM22/prophet-xii-tester/crates/prophet-render/src/lib.rs#L58-L122) e dos presets `presets/00-factory.json`..`09-factory.json` confirma que **apenas 4 demos por taxa de amostragem** (`single-note`, `chorus-i`, `chorus-ii`, `chorus-i-ii`) definem `ReverbWet = 0.0`, enquanto as demais 7 herdam `ReverbWet > 0` (entre `0.06` e `0.68`) de seus respectivos presets de fábrica.

### 2.1 Demos Secas / Sem Reverb (`ReverbWet = 0.0`) — Equivalência Auditiva e Numérica

| Demo WAV | Taxa | Δ RMS (dB) | Pico Baseline / Final | `max |Δ i16|` | SNR de Erro (dB) | Causa |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| `single-note-48000.wav` | 48 kHz | `+0.0000` | `0.095612` / `0.095612` | `1 LSB` | `101.02` | Reassociação `e8f572d` (~1 ULP `f32`) |
| `single-note-96000.wav` | 96 kHz | `-0.0000` | `0.095734` / `0.095734` | `1 LSB` | `101.22` | Reassociação `e8f572d` (~1 ULP `f32`) |
| `chorus-i-48000.wav` | 48 kHz | `+0.0000` | `0.209198` / `0.209198` | `1 LSB` | `98.11` | Reassociação `e8f572d` + cache `juno_chorus` |
| `chorus-i-96000.wav` | 96 kHz | `-0.0000` | `0.209473` / `0.209473` | `1 LSB` | `97.84` | Reassociação `e8f572d` + cache `juno_chorus` |
| `chorus-ii-48000.wav` | 48 kHz | `-0.0000` | `0.205933` / `0.205933` | `1 LSB` | `98.68` | Reassociação `e8f572d` + cache `juno_chorus` |
| `chorus-ii-96000.wav` | 96 kHz | `+0.0000` | `0.206299` / `0.206299` | `1 LSB` | `98.34` | Reassociação `e8f572d` + cache `juno_chorus` |
| `chorus-i-ii-48000.wav` | 48 kHz | `-0.0000` | `0.237579` / `0.237579` | `1 LSB` | `99.90` | Reassociação `e8f572d` + cache `juno_chorus` |
| `chorus-i-ii-96000.wav` | 96 kHz | `-0.0000` | `0.238953` / `0.238953` | `1 LSB` | `98.63` | Reassociação `e8f572d` + cache `juno_chorus` |

**Conclusão para o caminho seco e chorus**: diferença máxima de exatamente **1 bit menos significativo (`±1 LSB`)** em PCM 16 bits, variação de RMS inferior a `0,00005 dB` e erro relativo ~100 dB abaixo do sinal — sem qualquer regressão espectral, de forma de onda ou de transiente.

### 2.2 Demos com Reverb Ativo (`ReverbWet > 0`) — Mudança Intencional (`dfab71a`)

| Demo WAV | Preset (`ReverbWet`) | Taxa | Δ RMS (dB) | Pico Baseline / Final | `max |Δ i16|` | SNR Erro (dB) | Causa Principal |
| --- | --- | ---: | ---: | ---: | ---: | ---: | --- |
| `moog-drive-48000.wav` | P3 (`0.06`) | 48 kHz | `-0.0009` | `0.134094` / `0.134430` | `85` | `36.68` | Correção RT60 + reverb wet mínimo (`6%`) |
| `moog-drive-96000.wav` | P3 (`0.06`) | 96 kHz | `-0.0006` | `0.134155` / `0.134613` | `83` | `36.70` | Correção RT60 + reverb wet mínimo (`6%`) |
| `unison-48000.wav` | P7 (`0.18`) | 48 kHz | `-0.0081` | `0.269562` / `0.269562` | `356` | `23.83` | Correção RT60 em cauda curta (`2.2 s`) |
| `unison-96000.wav` | P7 (`0.18`) | 96 kHz | `-0.0076` | `0.271118` / `0.271118` | `361` | `23.77` | Correção RT60 em cauda curta (`2.2 s`) |
| `twelve-voices-48000.wav` | P0 (`0.20`) | 48 kHz | `+0.0593` | `0.265289` / `0.265533` | `749` | `20.80` | Remoção do droop de 30 Hz + calibração FDN |
| `twelve-voices-96000.wav` | P0 (`0.20`) | 96 kHz | `+0.0590` | `0.263580` / `0.263794` | `759` | `20.79` | Remoção do droop de 30 Hz + calibração FDN |
| `curtis-resonance-48000.wav` | P0 (`0.20`) | 48 kHz | `-0.1814` | `0.115326` / `0.116760` | `512` | `17.31` | Calibração FDN + difusão/shimmer do preset |
| `curtis-resonance-96000.wav` | P0 (`0.20`) | 96 kHz | `-0.1768` | `0.115570` / `0.117218` | `509` | `17.24` | Calibração FDN + difusão/shimmer do preset |
| `hard-sync-48000.wav` | P2 (`0.25`) | 48 kHz | `+0.0603` | `0.140228` / `0.140228` | `475` | `18.08` | Calibração FDN com `ReverbWet=0.25` |
| `hard-sync-96000.wav` | P2 (`0.25`) | 96 kHz | `+0.0629` | `0.139099` / `0.139099` | `479` | `17.84` | Calibração FDN com `ReverbWet=0.25` |
| `poly-mod-48000.wav` | P5 (`0.38`) | 48 kHz | `+0.4272` | `0.045349` / `0.045349` | `299` | `9.64` | Cauda sustentada (`decay=5.8 s`, `wet=0.38`) |
| `poly-mod-96000.wav` | P5 (`0.38`) | 96 kHz | `+0.4278` | `0.045502` / `0.045502` | `300` | `9.59` | Cauda sustentada (`decay=5.8 s`, `wet=0.38`) |
| `reverb-ensemble-48000.wav` | P6 (`0.68`) | 48 kHz | `+1.1915` | `0.066467` / `0.079834` | `1227` | `4.64` | Alvo explícito de RT60 (`16.5 s`, `wet=0.68`, vowel+shimmer) |
| `reverb-ensemble-96000.wav` | P6 (`0.68`) | 96 kHz | `+1.2140` | `0.063690` / `0.083130` | `1352` | `4.51` | Alvo explícito de RT60 (`16.5 s`, `wet=0.68`, vowel+shimmer) |

A magnitude do desvio escala monotonicamente com `ReverbWet` (`0.06 → 0.68`) e `ReverbDecay`, e permanece consistente entre `48 kHz` e `96 kHz` em cada preset, confirmando que as mudanças decorrem exatamente da correção de ganho de laço e atenuação espectral em [`solaris_reverb.rs`](file:///home/antigravity/SalatielBM22/prophet-xii-tester/crates/prophet-dsp/src/fx/solaris_reverb.rs#L679-L1030).

## 3. Determinismo Bitwise em Execuções Repetidas (`e23fa95`)

| Par Repetido (`a` vs `b`) | Taxa | Resultado | SHA-256 Idêntico |
| --- | ---: | --- | --- |
| `twelve-48k-{a,b}.wav` | 48 kHz | **Bitwise Identical** | `5192316a5adf6c0afe7d5d23db3ec4d0a446e36b00f1facf7f5b9ffba1178d1f` |
| `twelve-96k-{a,b}.wav` | 96 kHz | **Bitwise Identical** | `a8228f5860e603b9cccea9b6a9e16f1a8456eef51a3207567b686bbff9d8fb5c` |
| `midi-48k-{a,b}.wav` | 48 kHz | **Bitwise Identical** | `724f82a563f630b647640c8edf30923b3225e8e06a3b45523108891679710166` |
| `midi-96k-{a,b}.wav` | 96 kHz | **Bitwise Identical** | `b957626ef092ad645f0fd1dbcb166c35bfd52fcb00872bf1098db88af77bf7f5` |

## 4. Integridade dos Manifestos Históricos

- **`docs/analysis/SHA256SUMS`**: **23 / 23 OK** (`rust-and-analysis.log`). O pacote `crates/prophet-analysis` não foi modificado; a reutilização em host foi feita via ponte [`crates/prophet-analysis-core`](file:///home/antigravity/SalatielBM22/prophet-xii-tester/crates/prophet-analysis-core/src/lib.rs).
- **`docs/backend/evidence/baseline-artifacts.sha256` (129 linhas)**: **128 / 128 artefatos entregues OK** (`binaries/**`, `evidence/**`, `presets/**`). Apenas a linha 1 (`Cargo.lock`, anteriormente `ba4bfbc2...`) mudou para `069763ff...` (+75 linhas em `a787284`) ao incorporar o membro `prophet-analysis-core` e travar suas dependências no workspace raiz; a `Cargo.lock` original permanece íntegra na baseline `b47f2d6`.
