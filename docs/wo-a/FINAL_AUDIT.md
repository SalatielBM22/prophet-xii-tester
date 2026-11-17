# Auditoria do checkpoint para migração de ambiente

Estado geral **PARTIAL**. A implementação foi pausada a pedido do proprietário para preservar o trabalho e permitir a retomada em outra máquina. Incidente relatado e roteiro em [CLOUD_RESUME.md](CLOUD_RESUME.md). A autoridade para publicação abrange o branch `feat/dual-xhigh-core-dsp-vector` no fork `SalatielBM22/prophet-xii-tester`.

| Ponto | SHA |
|---|---|
| RC1 | `2978fdc02c85aa413dc76cc468cebb05f5cecc2a` |
| Contrato inicial | `995d772d7371480900fe74def6bb3c60674abddc` |
| Teste red Unison | `03a58bf483fb695b7380d7481c517f8a0e7ba5f1` |
| Controle, MIDI, Glide e Unison | `3c916e220e6efb533bc9bcecba5215477acd71d7` |
| FIR no_std, análise de validade e políticas de ganho | `2eb7ebcade6ceebe639815dc49dde4bf22267da9` |
| Código do checkpoint e rack FX | `f186ea10d752092d1d0b8ebc7c44226398f7d124` |

Os arquivos de áudio, presets, binários e código de hosts são idênticos à árvore RC1. O catálogo histórico `docs/backend/evidence/baseline-artifacts.sha256` registrava 128 arquivos coincidentes de 129 já na RC1. A divergência era o hash histórico do `Cargo.lock`. O checkpoint preserva o `Cargo.lock` da RC1.

## Decisões e limites de implementação

- `NUM_PARAMS` segue 83 e ID 83 é uma extensão separada. O banco RC1 segue o tempo quando não há enable explícito. `PreparedEngine` oferece validação de blocos, deduplicação por origem e eventos ordenados. O snapshot wire completo e negociação de capacidades ainda exigem integração. Não publique uma capability que o adapter ainda não implementa.
- O preset da RC1 e a API legada usam saída LegacyRC1. A preparação v1 usa FloatBus sem hard clip. O ganho opcional pós-VCF multiplica pelo Drive e permanece desligado por padrão. A adequação acústica e a margem de pico ainda precisam de comparação quantitativa.
- O classificador VCF usa uma execução com entrada zero e dois impulsos escalados em instâncias novas, com teto de 65.536 amostras por execução, cancelamento cooperativo e curva só para a classe Measurable. Os limiares são heurísticos. Rever os casos de sinal autônomo, DC e caudas longas antes de tratar as classes como evidência física. O host deve limitar concorrência.
- O reverb compartilha um banco de delays entre perfis. DiffuseV1 aumenta a componente lateral do Ensemble, reduz a mistura de Shimmer na realimentação de 0,28 para 0,14 vezes o amount e adiciona componente comum ao readout do Shimmer. O perfil começa desativado. Nenhuma alegação de melhora perceptiva. O corte agudo é limitado a 0,48×Fs para manter a prewarping TPT abaixo de Nyquist em taxas baixas.
- Gates dependentes de hardware, áudio perceptual, perfil de callback, aliasing e medição espectral específica continuam OPEN. Nenhum teste físico foi executado. [IMPLEMENTATION_MATRIX.md](IMPLEMENTATION_MATRIX.md) tem o status por issue e [BENCHMARKS.md](BENCHMARKS.md) tem a reprodução.
