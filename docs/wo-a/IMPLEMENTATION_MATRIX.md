# Matriz de continuidade

| Gate da ordem A | Estado | Evidência e trabalho restante |
|---|---|---|
| G0 RC1 e contrato | PASS / PROVEN / HOST_VERIFIED | RC1 `2978fdc02c85aa413dc76cc468cebb05f5cecc2a` verificada no fork. 37 testes de referência. Contrato publicado em `995d772d7371480900fe74def6bb3c60674abddc`. Aceite do Gemini B permanece OPEN. |
| G1 RC2 P0 | PARTIAL / PROVEN / HOST_VERIFIED | Glide 83 funciona sem renumerar 0–82. Unison aplica contagem e rampas nos membros retidos. Note ledger, sustain/sostenuto, ACKs e replay com testes. FloatBus e compensação opcional de Drive implementados. Classificador VCF tem testes sintéticos. Faltam matriz de taxas e blocos, sinais adversariais, teste do ponto 27,51 Hz / res 1 / drive 1,1 / morph 0,35, A/B Drive com RMS/peak/true-peak/THD/IMD, análise de alias e verificação física. |
| G2 núcleo reutilizável | PARTIAL / PROVEN / HOST_VERIFIED | `PreparedEngine` e extração sem alteração aritmética do FIR para o rlib `no_std`. Builds em x86_64 Linux, AArch64 Linux e AArch64 Android. Portar os demais módulos progressivamente, medir domínio de memória e obter paridade em cada etapa. |
| G3 reverb | PARTIAL / PROVEN / HOST_VERIFIED | `ReverbPort`, `LegacyRC1`, `DiffuseV1` com seleção apenas em tanque ocioso. Paridade LegacyRC1 nos renders fixados. Faltam testes e métricas próprios de DiffuseV1 e integração de seleção/tail/state via EnginePort. |
| G4 Classic2/Extended3/Extended4/Vector4 | BLOCKED / OPEN / NOT_TESTED | Implementação de topologias novas pendente no checkpoint. Classic2 passou comparação de WAV. Manter A+B original, adicionar C/D sem duplicar o VCF/VCA. Definir rotas tipadas áudio/FM/PM/sync/ring sem dependências cíclicas, fontes próprias banda-limitadas e envelopes XY de cinco estágios por voz. Validar pesos do diamante, suavização, loudness, alias, continuidade e 12 vozes em 48/96 kHz a 1x/2x/4x. |
| G5 sequenciador, arpejo e presets | BLOCKED / OPEN / NOT_TESTED | Contrato de IDs/migração proposto. Implementar relógio Rust e fila fixa, 32 passos em páginas de oito, 64 padrões, gate, ratchet, rests, ties, acento e glide por passo, busca/loop/stop e independência das fontes. Arpejo Up/Down/As-played/Random com seed e demais modos da issue #8. Implementar codec versionado, algoritmo/revisão, checksum, validação antes de aplicar e migração RC1, sem importadores proprietários. |
| G6 qualificação e integração | PARTIAL / OPEN / HOST_VERIFIED | Evidência de regressão de host e checkpoint publicado. Faltam matriz RT, alocações por callback, deadline e dispositivos. B coordena integração em branch separado. |

## Issues do fork

Todas as 15 issues apareciam abertas na lista GitHub consultada. Os estados abaixo classificam a entrega A e sua evidência local.

| Issue | Estado | Evidência |
|---|---|---|
| #1 Glide/MIDI | PARTIAL · PROVEN · HOST_VERIFIED | `3c916e2`, testes `events` e `port`. CASIO, caminhos reais e geometria ficam com B. |
| #2 validade VCF | PARTIAL · PROVEN · HOST_VERIFIED | `2eb7ebc`, `validity::tests::known_gain_and_invalid_regimes`. Falta reproduzir o ponto reportado, oracle independente e adaptador do host. |
| #3 Drive/headroom | PARTIAL · PROVEN · SOURCE_INSPECTED | `2eb7ebc`, opções de ganho no código. Falta caracterização para decidir a política de produção. |
| #4 integridade numérica | PARTIAL · PROVEN · HOST_VERIFIED | Testes e renders registrados em [BENCHMARKS.md](BENCHMARKS.md). Matriz DAW/alias/RT OPEN. |
| #5 React | BLOCKED · OPEN · NOT_TESTED | Escopo B. |
| #6 Ensemble/Shimmer | PARTIAL · PROVEN · SOURCE_INSPECTED | `f186ea1`, perfil DiffuseV1 experimental. Ensaios próprios OPEN. |
| #7 presets | PARTIAL · OPEN · SOURCE_INSPECTED | Migração descrita em [CONTROL_STATE_SCHEMA.md](CONTROL_STATE_SCHEMA.md). Codec e recall pendentes. |
| #8 sequenciador/arpejo | BLOCKED · OPEN · NOT_TESTED | Implementação suspensa antes deste gate. |
| #9 Unison | PARTIAL · PROVEN · HOST_VERIFIED | Teste red em `03a58bf`, green em `3c916e2`. Medir clicks e interações de transformação de pilha com MPE. |
| #10 roteamento VCO | BLOCKED · OPEN · NOT_TESTED | Implementação suspensa antes deste gate. |
| #11 Vector4 | BLOCKED · OPEN · NOT_TESTED | Contrato XY presente. Implementação suspensa antes deste gate. |
| #12 portas de reverb | PARTIAL · PROVEN · HOST_VERIFIED | `f186ea1`, paridade dos renders 48 kHz e 13 testes Legacy. |
| #13 no_std | PARTIAL · PROVEN · HOST_VERIFIED | `995d772`, `2eb7ebc`, build do rlib nos três alvos. Restante do DSP ainda usa std. |
| #14/15 hosts, plugins e Android | BLOCKED · OPEN · NOT_TESTED | Escopo B. Build do rlib para Android não representa execução de AAudio. |
