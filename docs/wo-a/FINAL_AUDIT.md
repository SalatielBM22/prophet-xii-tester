# Auditoria do candidato A

**Estado geral PARTIAL.** O candidato está disponível para integração experimental no fork. Classic2 e LegacyRC1 permanecem os padrões. O critério de zero perdas de prazo falhou na máquina Cloud. A promoção das topologias e do algoritmo de reverb depende dos gates abertos na [matriz por issue](IMPLEMENTATION_MATRIX.md).

| Identidade verificada | SHA |
|---|---|
| RC1 | `2978fdc02c85aa413dc76cc468cebb05f5cecc2a` |
| Contrato EnginePort v1 / MIDI / estado | `995d772d7371480900fe74def6bb3c60674abddc` |
| Checkpoint retomado | `9fb6825477ee8dbbd19119a86f6defabe10501b9` |
| Implementação publicada para B | `1f655c92f60a6a6124e7459014447f51f7d21517` |
| Commit de resultados confirmado em origin | `938247067a4b697e196512ce2d05991828a99b26` |

A verificação inicial encontrou o branch remoto exatamente no checkpoint. A RC1 é ancestral do contrato, o contrato é ancestral do checkpoint e a entrega descende do checkpoint. O worktree original de pesquisa permaneceu separado. O ZIP de referência tinha SHA-256 `c156f7236610bac4bd6de10e256e8192f7b2b99857162a41410d626b6c4bd6ac`; os 11 itens de SHA256SUMS.txt passaram. A ordem B avulsa coincidiu com a versão do pacote e serviu como referência de propriedade.

**Regressões.** Os 42 testes originais passaram antes da implementação. Na revisão publicada passaram 55 testes release dos cinco crates de núcleo/render e 2 do crate de presets, incluindo os 14 de reverb. [Resultados por teste](evidence/pinned-tests.txt). Os 11 hashes dos WAVs Classic2/LegacyRC1 pela API legada coincidiram com a RC1 recompilada e o checkpoint. Os dez arquivos de preset RC1 foram decodificados e preparados em 44,1/48/96 kHz. O contrato Rust e os documentos wire, fixture e mapa de IDs ficaram idênticos aos da revisão inicial do contrato.

**Portabilidade e propriedade.** O rlib no_std compilou para x86_64 Linux, AArch64 Linux e AArch64 Android. O check do workspace Linux e o Clippy dos crates de A concluíram com avisos. O diff da retomada está limitado aos crates de A, `tests/dsp-qualification` e `docs/wo-a`. Hosts, Android, UI, presets de fábrica, binários RC1, manifest e lock da raiz ficaram idênticos à RC1. Os crates de preset e qualificação têm locks separados.

**Decisão de desempenho.** O teste de 480 frames encontrou zero chamadas alloc/free no processamento das 108 configurações e 34 blocos fora do prazo. O ensaio não cobre todos os fluxos possíveis de comandos e não instrumenta syscalls de I/O ou bloqueios de kernel. A inspeção dos novos caminhos de render, relógio e roteamento encontrou buffers limitados e dados preparados fora do callback. No benchmark Classic2 de 256 frames, a mediana das diferenças relativas de tempo mediano foi +4,02% frente à RC1. [Condições, CSVs e reprodução](BENCHMARKS.md).

**Pendências para integrar.** B deve registrar aceite no SHA do contrato, combinar as capacidades com as do host e qualificar recall, automação, UI, múltiplas instâncias e dispositivos. O A/B humano de reverb, a varredura espectral das rotas moduladas e a qualificação temporal no alvo continuam OPEN. As amostras A/B e o procedimento estão nos benchmarks. A extração no_std continua incremental; os filtros, efeitos e preparação ainda precisam de validação por módulo. [Contrato para B](ADAPTER_DELTA.md).
