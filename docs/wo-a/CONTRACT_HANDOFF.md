# Gemini B adapter acceptance

Pin the first commit on this branch, titled `feat(contract): publish EnginePort v1 and RC1 migration (#1 #7 #13)`. Its published SHA is `995d772d7371480900fe74def6bb3c60674abddc`. Baseline is RC1 `2978fdc02c85aa413dc76cc468cebb05f5cecc2a`.

B records acceptance against the exact contract SHA in `docs/wo-b/CONTRACT_ADAPTER_AUDIT.md`. Verify the fixture command sequence, parameter map, rejected-block behavior, sample offsets, exclusive physical-device ownership, independent instance state, latency/tail and RC1 migration. Route every host through the typed command stream and publish snapshots off the audio thread. Missing capabilities disable that control. UI adapters share wire semantics; the native and embedded paths run offline. B owns platform and device qualification.

Process requests in `host-requests.json`. Return incompatible requirements via `docs/wo-b/CONTRACT_CHANGE_REQUESTS.md`. B reconciles workspace, lockfile, shared generated assets and adapters on the separate integration branch.


## Revisão publicada após a retomada

O pin de implementação para B é `1f655c92f60a6a6124e7459014447f51f7d21517`. O contrato inicial permanece em `995d772d7371480900fe74def6bb3c60674abddc`. Veja [ADAPTER_DELTA.md](ADAPTER_DELTA.md) para as APIs Rust e o protocolo de aceite, [BENCHMARKS.md](BENCHMARKS.md) para a reprodução e [IMPLEMENTATION_MATRIX.md](IMPLEMENTATION_MATRIX.md) para os gates em aberto. B deve validar o transporte, os limites combinados de fila, a troca de instância no recall e a remoção de capacidades que o host ainda não oferece.

## Histórico do checkpoint de migração (9fb6825)

- Importe `prophet_control::{PreparedEngine, port}`. Use `port::EnginePort` para `prepare(Prepare)` e `process(left, right, commands, acks)`. Origem, serial e offsets precisam ser validados antes de atravessar a fronteira da thread de áudio. Use um buffer de ACK com uma entrada por comando, inclusive para rejeições individuais e replays.
- Consulte `configuration()`, `latency_samples()`, `revision()`, `rendered_frames()`, `parameter(id)`, `notes()` e `telemetry()`. Estado das teclas vem de `Note.pressed`. Uma entrada retida com `pressed == false` está sob pedal. Telemetria de VCA e ADSR é independente do estado de tecla. O levantamento do pedal ou desconexão deve entrar como comando da origem correspondente.
- Mantenha o array RC1 de 83 entradas e ofereça p83 como automação adicional quando o adapter puder persistir o campo. `OversamplingMode` mantém seu ID e deve ser aplicado via prepare fora do callback. Um comando que solicita mudar esse parâmetro durante process recebe Unsupported. A adaptação v1 usa a saída FloatBus. A API de host RC1 continua disponível para paridade.
- `prophet_analysis_core::validity::measure` roda no worker. Só desenhe os pontos quando retornar Measurable. As demais classes retornam pontos vazios. Limite concorrência e descarte pedidos superseded. O schema inicial ainda não inclui o payload desse endpoint.
- `prophet_dsp::fx::reverb::{ReverbPort, ReverbSlot}` oferece os perfis e status DSP. A API v1 ainda precisa expor seleção e snapshot FX completo. A ativação de DiffuseV1 aguarda a qualificação. VectorXY ainda retorna Unsupported. Scheduler, persistência nativa e negociação completa ainda estão OPEN.
- O aceite do lado B deve verificar isolamento entre instâncias, empates ordenados, tamanhos variáveis com 480 frames, recuperação de origem, flags de capacidade, migração e separação entre latência do motor e do dispositivo. Registre o SHA inicial e o SHA do código que o adapter usa.

O roteiro de recuperação do Rust e Git está em [CLOUD_RESUME.md](CLOUD_RESUME.md). A classificação técnica por issue está em [IMPLEMENTATION_MATRIX.md](IMPLEMENTATION_MATRIX.md).
