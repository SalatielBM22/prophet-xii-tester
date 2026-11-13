# Validação FFT, WASM e Transferência VCF (G8.4)

Este relatório comprova a eliminação da medição legada DTFT fixa a `48 kHz` no endpoint `POST /api/v1/filter` (`a787284`), substituindo-a pela análise FFT de resposta ao impulso baseada na biblioteca [`prophet-analysis`](file:///home/antigravity/SalatielBM22/prophet-xii-tester/crates/prophet-analysis/src/transfer.rs) na taxa real do host, além da verificação completa da cadeia PCM → WebAssembly Worker → UI em `48 kHz` e `96 kHz`.

## 1. Substituição do Endpoint `/api/v1/filter` por `TransferAnalyzer` FFT + `RequestClock`

- **Ponte sem modificação da workspace independente**: [`crates/prophet-analysis-core`](file:///home/antigravity/SalatielBM22/prophet-xii-tester/crates/prophet-analysis-core/src/lib.rs) expõe `spectrum`, `stream` e `transfer` (`RequestClock`, `RequestToken`, `TransferAnalyzer`, `log_frequencies`) para o workspace raiz sem alterar nenhum dos 24 arquivos protegidos por `docs/analysis/SHA256SUMS`.
- **Execução na taxa de amostragem do host**: [`compute_filter_transfer`](file:///home/antigravity/SalatielBM22/prophet-xii-tester/crates/prophet-standalone/src/lib.rs#L463-L535) instancia `DualVcfStage::new(sample_rate, 1.0)` na taxa real do servidor (`48.000`, `96.000`, etc.) e gera grade logarítmica de `20 Hz` até `min(0.49 * fs, 20.000 Hz)` em 256 pontos.
- **Extensão adaptativa de cauda impulsiva**: inicia em `4.096` amostras e dobra progressivamente até `131.072` amostras (`FFT 262.144`) caso a energia RMS da oitava final ultrapasse `1e-4` da energia média (filtros de Q alto próximo a auto-oscilação), devolvendo `fftSize`, `impulseSamples`, `tailConverged`, `tailRelativeRms` e `binHz` — com terminação segura garantida mesmo em `Q = 0,98` (auto-oscilação).
- **Preempção de requisições obsoletas e limite de concorrência**: [`filter_response`](file:///home/antigravity/SalatielBM22/prophet-xii-tester/crates/prophet-standalone/src/main.rs#L224-L264) utiliza [`RequestClock`](file:///home/antigravity/SalatielBM22/prophet-xii-tester/crates/prophet-standalone/src/main.rs#L37-L38) e `Semaphore(2)`, abortando cálculos superados e retornando `HTTP 409 Conflict` (`{"superseded": true}`), tratado explicitamente pela fila [`TransferQueue`](file:///home/antigravity/SalatielBM22/prophet-xii-tester/ui/src/analysis/AnalysisPanel.tsx#L112-L125) no frontend sem poluir estado ou callback de áudio.
- **Legenda e unidade corretas na UI**: [`formatFilterCaption`](file:///home/antigravity/SalatielBM22/prophet-xii-tester/ui/src/analysis/AnalysisPanel.tsx#L42-L56) informa a taxa real de amostragem, o tamanho exato da FFT, número de amostras do impulso, resolução `Δf` em Hz e status de convergência da cauda, mantendo a distinção clara para hosts legados (`legacy`).

## 2. Evidências de Teste Verificadas (`e23fa95`)

### 2.1 Testes Unitários e de Integração Rust + Node WASM

- **Integração Backend [`filter_transfer.rs`](file:///home/antigravity/SalatielBM22/prophet-xii-tester/crates/prophet-standalone/tests/filter_transfer.rs#L17-L181)** (**4/4 PASS**):
  - `transfer_respects_sample_rate_cutoff_rolloff_and_phase_mask`: valida `48 kHz` e `96 kHz` em Curtis (`morph=0`) e Moog (`morph=1`), atenuação `> 40 dB` na banda de rejeição e máscara de fase `< -100 dB`.
  - `transfer_adaptively_extends_high_q_tail_and_handles_self_oscillation`: confirma extensão além de `4.096` com convergência (`tail_relative_rms < 1e-4`) em `Q = 0,85` e terminação finita sem panic em `Q = 0,98` (`131.072` amostras / `262.144` FFT).
  - `transfer_respects_superseded_request_clock`: confirma retorno imediato `Err(Error::Superseded)`.
  - `transfer_semaphore_bounds_concurrency_and_supersedes_burst`: rajada de 8 tarefas paralelas mantendo pico concorrente `≤ 2` e garantindo que a última requisição nunca seja descartada.
- **`prophet-analysis --features web`**: **18/18 PASS** (`2` unit + `15` accuracy + `1` alocação zero pós-planejamento).
- **WASM Runtime Node v24.22.2 (`web_runtime.mjs`)**: **27 cenários PASS** (`44.1/48/96 kHz`, tamanhos `2048/4096/8192`, calibração dBFS de Hann/Blackman-Harris/Kaiser, gaps/pacotes fora de ordem, ponteiros estáveis e refresh de typed-array pós `memory.grow`; WASM SHA-256 `275815492426c4bd4ab5c76050aef594e23f32301bd1205c38232ec1f8be3c1d`).

### 2.2 Suíte End-to-End Playwright Real (Worker WASM + Backend Nativo)

Extraído diretamente de `/tmp/g8-evidence/final/e2e-48k/e2e-native.json` e `/tmp/g8-evidence/final/e2e-96k/e2e-native.json`:

| Caso E2E | 48 kHz (`/tmp/g8-evidence/final/e2e-48k`) | 96 kHz (`/tmp/g8-evidence/final/e2e-96k`) | Observações Medidas |
| --- | --- | --- | --- |
| `native_capture` | **PASS** (`0.953 s`, 100 blocos, 5.866 amostras ≠ 0) | **PASS** (`1.025 s`, 176 blocos, 10.996 amostras ≠ 0) | Gap injetado (`gapCount: 1`), chunk ≤ 64 amostras |
| `native_spectrum` | **PASS** (`3.888 s`): 220 Hz → `222.66 Hz` (Hann 4096 / BH 8192); 440 Hz → `439.45 Hz` (BH 8192) e `445.31 Hz` (Kaiser 2048) | **PASS** (`3.726 s`): 220 Hz → `210.94` / `222.66 Hz`; 440 Hz → `445.31` / `421.88 Hz` | Drop de pacote e fechamento de WebSocket (code `4001`) geram reinício limpo e contador `discontinuities` estritamente monotônico (`0→5→8→9→10`) |
| `native_transfer` | **PASS** (`2.137 s`): `fftSize=65536`, `impulse=32768`, `Δf=0.732 Hz`, `tailConverged=true` (`8.94e-6`), `251` visíveis + `5` mascarados | **PASS** (`2.261 s`): `fftSize=131072`, `impulse=65536`, `Δf=0.732 Hz`, `tailConverged=true` (`8.93e-6`), `256` pontos | Coalescência de 10 edições → `2` requisições; `maxConcurrent=1`; `sampleRate` medido igual ao host |

### 2.3 Verificação do Proxy de Desenvolvimento Vite (`vite-proxy-probe.json`)

- Proxy Vite (`http://127.0.0.1:4173`) repassando para backend nativo (`http://127.0.0.1:8792`):
  - **HTTP PASS**: `/api/v1/meta` (`83` parâmetros, `10` presets), `/api/v1/state` (`48000 Hz`), `/api/v1/filter` (`sampleRate=48000`, `fftSize=65536`, `impulseSamples=32768`, `tailConverged=true`, `256` pontos).
  - **WebSocket PASS**: `/api/v1/ws` (frames `3074048 → 3076096`) e `/api/v1/capture` (`4` blocos contíguos a `48000 Hz`, sequências `48076..48079`).
