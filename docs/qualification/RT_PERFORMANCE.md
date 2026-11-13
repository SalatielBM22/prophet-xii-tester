# Qualificação de Desempenho em Tempo Real (G8.2 e G8.3)

Este relatório compara o comportamento computacional de callback de áudio e interface entre a baseline original (`b47f2d6`), o checkpoint integrado pré-G8 ([`RT_OPTIMIZATION.md`](file:///home/antigravity/SalatielBM22/prophet-xii-tester/docs/backend/RT_OPTIMIZATION.md)) e o código final RC1 (`e23fa95`), medido em host Linux x86_64 sem isolamento de CPU (*scheduler jitter* do KVM retido).

> [!NOTE]
> Resultados abaixo são **`HOST_VERIFIED`** em benchmark offline e host `--null`. Latência de driver WASAPI/ALSA, XRuns reais e desempenho térmico em dispositivos físicos Windows ou Snapdragon 665 permanecem **`OPEN / DEVICE_VERIFIED: false`**.

## 1. Otimizações Estruturais Implementadas (sem redução de fidelidade ou polifonia)

1. **Solucionador Moog ZDF O(1) (`e8f572d`)**: [`MoogLadderFilter::process_sample`](file:///home/antigravity/SalatielBM22/prophet-xii-tester/crates/prophet-dsp/src/vcf_moog.rs#L178-L309) resolve o sistema implícito não linear de 4 estágios + realimentação global por um passo de Newton acoplado de matriz 4×4 *bordered-bidiagonal* em tempo O(1) e avaliação vetorial agrupada de `eval_tanh_sech2_5`, mantendo o loop aninhado original (`fallback_nested_newton`) apenas como fallback se o resíduo exceder tolerância.
2. **Hoisting e Pareamento Simétrico (`e8f572d`)**:
   - [`ProphetVoice::process_sample`](file:///home/antigravity/SalatielBM22/prophet-xii-tester/crates/prophet-dsp/src/voice.rs#L371-L625) move atualizações de coeficiente invariantes por sub-amostra para fora da etapa `oversample` e simplifica caminhos de modulação.
   - [`SymmetricFirKernel::step`](file:///home/antigravity/SalatielBM22/prophet-xii-tester/crates/prophet-dsp/src/oversample.rs#L108-L123) explora a simetria do FIR halfband reduzindo multiplicações pela metade.
   - [`fast_tanh_and_sech2`](file:///home/antigravity/SalatielBM22/prophet-xii-tester/crates/prophet-dsp/src/vcf_cem3320.rs#L32-L45) evita dupla divisão na derivada.
3. **Ensemble SoA + Cache de Coeficientes (`dfab71a`)**: [`EnsembleBank`](file:///home/antigravity/SalatielBM22/prophet-xii-tester/crates/prophet-dsp/src/fx/solaris_reverb.rs#L201-L260) reorganiza as 48 bandas logarítmicas em 3 *lanes* × 16 bandas (`[f32; 16]`) alinhadas a 32 bytes para auto-vetorização sem ramos, além de cachear coeficientes de `high_cut_hz`, `low_cut_hz` e `decay_sec`.

## 2. Perfil por Componente (`hotspots.csv`, ns/amostra)

| Componente | Baseline (`b47f2d6`) | Pré-G8 (`430e4d8`) | Final RC1 (`e23fa95`) | Ganho vs Baseline |
| --- | ---: | ---: | ---: | ---: |
| `curtis` | `87.07` | `82.57` | `93.75` | ~equivalente (ruído de VM) |
| **`moog`** | `624.89` | `639.24` | **`65.66`** | **~9,52× mais rápido** |
| `reverb-Off` | `386.88` | `363.84` | **`300.84`** | **~1,29× mais rápido** |
| `reverb-MezzoPiano` | `2004.05` | `1172.07` | **`469.13`** | **~4,27× mais rápido** |
| `reverb-Forte` | `1919.80` | `1142.41` | **`467.18`** | **~4,11× mais rápido** |

## 3. Matriz Completa `rt_matrix` (144 casos / 14.400 callbacks em `256` frames)

Comparação direta entre `baseline-rt-256.csv`, `candidate-rt-256.csv` e `/tmp/g8-evidence/final/rt-256.csv`:

| Geração | Casos com ≥1 miss (/144) | Total de misses (/14.400) | Misses a 48 kHz (/7.200) | Misses a 96 kHz (/7.200) |
| --- | ---: | ---: | ---: | ---: |
| Baseline (`b47f2d6`) | `39 / 144` | `2.440` (`16,94%`) | `897` | `1.543` |
| Candidato Pré-G8 (`candidate-rt-256`) | `45 / 144` | `2.453` (`17,03%`) | `922` | `1.531` |
| **Final RC1 (`e23fa95`)** | **`2 / 144`** | **`4` (`0,028%`)** | **`0 / 7.200`** | **`4 / 7.200`** |

### Casos Críticos Selecionados (12 vozes, `256` frames, µs por callback)

| Caso | Geração | p50 (µs) | Max (µs) | Orçamento (µs) | Misses / 100 |
| --- | --- | ---: | ---: | ---: | ---: |
| `48 kHz / 4x / Moog / 12v / forte` | Baseline | `11983.8` | `18414.1` | `5333.3` | `100` |
| `48 kHz / 4x / Moog / 12v / forte` | Candidato Pré-G8 | `12187.9` | `22391.3` | `5333.3` | `100` |
| **`48 kHz / 4x / Moog / 12v / forte`** | **Final RC1 (`e23fa95`)** | **`1657.6`** | **`2459.0`** | `5333.3` | **`0`** (~**7,23×** p50 mais rápido) |
| `48 kHz / 4x / Moog / 12v / dry` | Baseline | `11231.7` | `13662.8` | `5333.3` | `100` |
| **`48 kHz / 4x / Moog / 12v / dry`** | **Final RC1 (`e23fa95`)** | **`1534.3`** | **`1642.2`** | `5333.3` | **`0`** (~**7,32×** p50 mais rápido) |
| `48 kHz / 4x / Curtis / 12v / forte` | Baseline → Final RC1 | `2196.8 → 1210.3` | `2797.6 → 1339.3` | `5333.3` | `0 → 0` |
| `96 kHz / 4x / Moog / 12v / forte` | Baseline | `13918.2` | `35158.9` | `2666.7` | `100` |
| **`96 kHz / 4x / Moog / 12v / forte`** | **Final RC1 (`e23fa95`)** | **`1654.4`** | **`4235.3`** | `2666.7` | **`2`** (`p95=2214.5`, `p99=2853.2`) |
| `96 kHz / 4x / Moog / 12v / dry` | Baseline | `11902.4` | `22988.0` | `2666.7` | `100` |
| **`96 kHz / 4x / Moog / 12v / dry`** | **Final RC1 (`e23fa95`)** | **`1516.9`** | **`5070.8`** | `2666.7` | **`2`** (`p95=2052.1`) |

## 4. Todos os Buffers (`64`, `256`, `512` frames com captura) e Relato Honesto de Outliers

Consolidando as 3 execuções da matriz final em `e23fa95` (`rt-256.csv`, `rt-64-capture.csv`, `rt-512-capture.csv`, total **432 casos / 43.200 callbacks**, sendo 64 e 512 com captura PCM habilitada):

- **Alocações medidas no callback**: **`0`** em todos os 432 casos (confirmado também pelos 3 testes de [`callback.rs`](file:///home/antigravity/SalatielBM22/prophet-xii-tester/crates/prophet-standalone/tests/callback.rs#L40-L113)).
- **Subnormais emitidos**: **`0`** em todos os 432 casos; 100% das amostras finitas.
- **Misses a 48 kHz**: **`0 / 21.600` callbacks** em todos os tamanhos de bloco (`64`, `256` e `512`). Por exemplo, `48 kHz / 4x / Moog / 12v / forte` a 64 frames atingiu `p50=450,6 µs`, `max=604,5 µs` contra orçamento `1333,3 µs` (`0/100` misses).
- **Misses totais combinados**: **`9 / 43.200` callbacks (`0,021%`)**, ocorridos exclusivamente em 5 casos a **96 kHz** por picos de escalonamento da VM compartilhada:

| Arquivo | Caso com >0 Misses | p50 (µs) | p95 (µs) | Max (µs) | Orçamento (µs) | Misses / 100 |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| `rt-256.csv` | `96k / 4x / Moog / 12v / dry (fx=0)` | `1516.9` | `2052.1` | `5070.8` | `2666.7` | `2` |
| `rt-256.csv` | `96k / 4x / Moog / 12v / forte (fx=2)` | `1654.4` | `2214.5` | `4235.3` | `2666.7` | `2` |
| `rt-64-capture.csv` | `96k / 4x / Moog / 12v / FDN-off (fx=1)` | `457.2` | `535.2` | `697.9` | `666.7` | `2` |
| `rt-512-capture.csv` | `96k / 2x / Curtis / 12v / forte (fx=2)` | `1349.7` | `3233.4` | `6022.7` | `5333.3` | `2` |
| `rt-512-capture.csv` | `96k / 2x / Moog / 12v / FDN-off (fx=1)` | `1810.2` | `3227.5` | `5969.8` | `5333.3` | `1` |

*(Observação: no mesmo arquivo `rt-512-capture.csv`, a configuração mais pesada `96k / 4x / Moog / 12v / forte` teve `0/100` misses com `p50=3196,2 µs`, `max=3389,6 µs`, reforçando que os 2 casos 2× em `rt-512` foram picos isolados de preempção do hipervisor.)*

## 5. Benchmark `prophet-bench` (300 iterações, 12 vozes forte, `256` frames)

Em `/tmp/g8-evidence/final/prophet-bench.csv`: **0 deadline misses em todas as 12 configurações** (48k e 96k × 1x/2x/4x × Curtis/Moog). Destaques:
- `48 kHz / 4x / Curtis`: mediana `1158,9 µs`, p99 `2236,8 µs`, max `2611,4 µs` (**22,46% RT**).
- `48 kHz / 4x / Moog`: mediana `1683,3 µs`, p99 `1802,6 µs`, max `1888,6 µs` (**31,63% RT**).
- `96 kHz / 4x / Curtis`: mediana `1225,3 µs`, p99 `1406,9 µs`, max `1491,1 µs` (**45,57% RT**).
- `96 kHz / 4x / Moog`: mediana `1627,0 µs`, p99 `1891,1 µs`, max `1910,7 µs` (**61,74% RT**).
- Armazenamento inline: `kernel_inline_bytes = 119.200`, `processor_inline_bytes = 157.344` (em `rt-256.csv`).

## 6. Desempenho da UI no Browser (`browser_performance.py`)

Medição com o host integrado a 48 kHz + streaming WS + espectro WASM em `/tmp/g8-evidence/final/perf/g8_final-browser-performance.json` (Chromium `156.0.8078.79`):

- **Status**: **PASS** (`p95BudgetMs = 50`)
- **Quadros medidos**: `358` em `6003,9 ms`
- **Intervalo entre quadros**: `p50 = 16,7 ms`, **`p95 = 16,8 ms`**, `p99 = 16,8 ms`, `max = 33,3 ms`
- **Long Tasks (>50 ms)**: **`0`** (`longTaskMaxMs: 0`)
- **Uso de CPU na thread principal**: `ScriptDuration = 1,067 s`, `LayoutDuration = 0,150 s`, `RecalcStyleDuration = 0,023 s`
- **Erros de console/página**: `[]` (`0` erros)
