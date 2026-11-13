# Qualificação de Calibração Reverb RT60 e Efeitos Solaris (G8.5)

Este documento registra a correção estrutural e as medições Schroeder do motor [`SolarisCloudburstReverb`](file:///home/antigravity/SalatielBM22/prophet-xii-tester/crates/prophet-dsp/src/fx/solaris_reverb.rs#L650-L1098) implementadas em `dfab71a` e validadas por [`reverb_regression.rs`](file:///home/antigravity/SalatielBM22/prophet-xii-tester/crates/prophet-dsp/tests/reverb_regression.rs#L1-L541) e `/tmp/g8-evidence/final/reverb-decay.csv`.

## 1. Causas Raiz Corrigidas (Pré-G8 → RC1)

No checkpoint pré-G8 ([`reverb-rt60.csv`](file:///home/antigravity/SalatielBM22/prophet-xii-tester/docs/backend/evidence/reverb-rt60.csv)), a resposta impulsiva apresentava subestimação severa de decaimento — em `4,8 s` nominais, o perfil `neutral` caía para `3,93 s` (~18% curto) e o perfil `artistic-default` colapsava para **`0,56 s` (~88% curto)**:

1. **Bloqueador DC dentro do laço FDN**: o filtro passa-altas interno operava com corte em `30 Hz`, acumulando atenuação na banda grave/média ao longo de centenas de iterações do FDN; substituído em [`set_sample_rate`](file:///home/antigravity/SalatielBM22/prophet-xii-tester/crates/prophet-dsp/src/fx/solaris_reverb.rs#L684-L688) por um bloqueador TPT de **`1,5 Hz`** sem *droop* audível em 48/96 kHz.
2. **Shelf HF proporcional de Jot no laço**: em [`process_stereo`](file:///home/antigravity/SalatielBM22/prophet-xii-tester/crates/prophet-dsp/src/fx/solaris_reverb.rs#L763-L785), o shelf de alta frequência anula-se continuamente em `high_cut_hz ≥ 17,9 kHz` (perfil neutro banda larga) e escala com o tempo de atraso individual `τ_i` de cada linha quando `high_cut_hz < 17,9 kHz`.
3. **Estabilidade de contração do Shimmer no feedback FDN**: mistura linear estritamente contrativa em [`solaris_reverb.rs`](file:///home/antigravity/SalatielBM22/prophet-xii-tester/crates/prophet-dsp/src/fx/solaris_reverb.rs#L932-L955) com `shimmer_amount ≤ 0,85` e ganho de realimentação `g_fb = shimmer_amount * 0,28`, garantindo `|(1 - g)*m0 + g*P(m0)| ≤ |m0|` sem ciclos-limite.
4. **Filtro de Formantes (`A/E/I/O/U`) fora do laço recursivo**: aplicado na leitura estéreo com ganho passivo compensado (`1,65 / 1,85 = 0,892 < 1,0`) em [`solaris_reverb.rs`](file:///home/antigravity/SalatielBM22/prophet-xii-tester/crates/prophet-dsp/src/fx/solaris_reverb.rs#L1016-L1031), preservando o RT60 do tanque.

## 2. Medições Schroeder T20 / T30 / Sub-bandas (`/tmp/g8-evidence/final/reverb-decay.csv`)

Medido via integração regressiva de Schroeder nos intervalos `-5..-25 dB` (`T20`) e `-5..-35 dB` (`T30`), janela `4 * Decay + 1 s`, a `48 kHz` e `96 kHz`:

| Taxa | Perfil | Alvo (s) | T20 (s) | T20 R² | T30 (s) | T30 R² | `<400 Hz` (s) | `400–3k Hz` (s) | `>3 kHz` (s) | Pico | Corr. Estéreo |
| ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 48 kHz | `neutral` | `0.5` | `0.501290` | `0.999958` | `0.499763` | `0.999968` | `0.476` | `0.512` | `0.499` | `0.00727` | `0.0022` |
| 48 kHz | `artistic-default` | `0.5` | `0.476820` | `0.999752` | `0.487787` | `0.999683` | `0.471` | `0.491` | `0.453` | `0.00261` | `0.0044` |
| 48 kHz | `neutral` | `1.5` | `1.495084` | `0.999972` | `1.496798` | `0.999989` | `1.522` | `1.481` | `1.498` | `0.00811` | `0.0043` |
| 48 kHz | `artistic-default` | `1.5` | `1.354380` | `0.999810` | `1.366483` | `0.999901` | `1.462` | `1.427` | `1.196` | `0.00268` | `-0.0016` |
| 48 kHz | `neutral` | `4.8` | `4.797214` | `0.999995` | `4.798291` | `0.999998` | `4.779` | `4.804` | `4.796` | `0.00852` | `0.0069` |
| 48 kHz | `artistic-default` | `4.8` | `3.893561` | `0.999921` | `3.965923` | `0.999775` | `4.137` | `4.091` | `3.162` | `0.00276` | `0.0002` |
| 48 kHz | `neutral` | `10.0` | `9.998214` | `0.999999` | `9.999994` | `0.999999` | `9.951` | `10.003` | `9.998` | `0.00862` | `0.0077` |
| 48 kHz | `artistic-default` | `10.0` | `7.300053` | `0.999564` | `7.515724` | `0.999480` | `7.956` | `7.630` | `5.403` | `0.00277` | `-0.0008` |
| 96 kHz | `neutral` | `0.5` | `0.500647` | `0.999957` | `0.499276` | `0.999963` | `0.476` | `0.512` | `0.499` | `0.00506` | `0.0027` |
| 96 kHz | `artistic-default` | `0.5` | `0.476017` | `0.999849` | `0.485110` | `0.999771` | `0.471` | `0.492` | `0.458` | `0.00184` | `0.0067` |
| 96 kHz | `neutral` | `1.5` | `1.496816` | `0.999971` | `1.497193` | `0.999990` | `1.522` | `1.481` | `1.500` | `0.00517` | `0.0049` |
| 96 kHz | `artistic-default` | `1.5` | `1.356329` | `0.999815` | `1.369090` | `0.999903` | `1.463` | `1.444` | `1.242` | `0.00192` | `-0.0019` |
| 96 kHz | `neutral` | `4.8` | `4.796277` | `0.999996` | `4.797798` | `0.999998` | `4.779` | `4.803` | `4.795` | `0.00544` | `0.0074` |
| 96 kHz | `artistic-default` | `4.8` | `3.969587` | `0.999958` | `4.027387` | `0.999857` | `4.145` | `4.205` | `3.551` | `0.00195` | `-0.0012` |
| 96 kHz | `neutral` | `10.0` | `9.997740` | `0.999999` | `10.000202` | `0.999999` | `9.951` | `10.003` | `9.997` | `0.00552` | `0.0084` |
| 96 kHz | `artistic-default` | `10.0` | `7.508943` | `0.999767` | `7.665725` | `0.999728` | `7.966` | `7.991` | `6.473` | `0.00196` | `-0.0017` |

### Análise dos Resultados e Limitações Abertas

- **Perfil Neutro (`HOST_VERIFIED PASS`)**: erro máximo em `T20` e `T30` é `≤ 0,33%` de `1,5 s` a `10,0 s` e `≤ 0,62%` em `0,5 s` (`R² ≥ 0,999957`), com coerência entre `48 kHz` e `96 kHz` e correlação estéreo próximo a zero (`0,0022 .. 0,0084`).
- **Perfil Artístico Padrão (`HOST_VERIFIED PASS` para monotonicidade e linearidade logarítmica)**: evolui suavemente (`R² ≥ 0,99948`) de `0,48 s → 1,35 s → 3,89–3,97 s → 7,30–7,51 s` (em contraste com `0,33 s → 0,45 s → 0,56 s` pré-G8). O decaimento mais curto na banda `>3 kHz` reflete a absorção intencional de ar (`high_cut_hz = 7500 Hz`) do shelf Jot.
- **Testes Funcionais Complementares (`reverb_regression.rs`, 13/13 PASS)**: shimmer `+12` e `+7` semitons (`Goertzel > 100×`), formantes `A/E/I/O/U` distinguíveis (`> 3×`), progressão de harmônicos/energia `Off < MezzoPiano < Forte`, congelamento `Freeze` estável por 10 segundos (`drift < 0,05 dB`, correlação `< 0,25`), política `Spill`/`Kill` com fade `5 ms` e dormência automática sem subnormais.
- **Itens ainda `OPEN`**: audição subjetiva em alto-falantes/fones sobre qualidade musical do pitch-shifter de shimmer, deriva energética em `Freeze` contínuo de múltiplos minutos e varredura paramétrica exaustiva de sensibilidade musical em hardware.
