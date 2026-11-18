# Medições da retomada

Revisão de entrega `1f655c92f60a6a6124e7459014447f51f7d21517`, RC1 `2978fdc02c85aa413dc76cc468cebb05f5cecc2a`. Máquina Cloud Linux x86_64 Intel Xeon Platinum 8573C, 3 CPUs visíveis, cota de 2 CPUs (`cpu.max = 200000 100000`), escalonamento comum. Rust/Cargo 1.100.0, NumPy 2.5.4, SciPy 1.18.1 e FFmpeg 7.1.5. Testes offline usam as revisões dos locks da raiz e de cada crate isolado. A carga de vizinhos da VM não foi medida.

## Paridade, memória e tempo de bloco

- [11 renders estéreo](evidence/resumed-wav-parity.csv), a 48 kHz por 1 s. Recompilação independente da RC1, último render do candidato e hashes históricos coincidem byte a byte.
- O mesmo executável de benchmark recompilado para cada revisão cobre 12 condições Classic2 de 12 vozes, 48/96 kHz, 1/2/4x, Curtis/Moog, Ensemble Forte, blocos de 256. O benchmark faz 30 blocos de aquecimento e mede 300 por condição. A mediana das 12 variações relativas do tempo mediano foi **+4,02%** (de −2,01% a +8,40%). Houve 28 perdas em 3.600 blocos na RC1 e 20 em 3.600 no candidato. No caso Moog 96 kHz/4x o p99 passou de 2,710 para 3,889 ms, com orçamento de 2,667 ms. Uma execução por revisão, sem inferência causal sobre diferenças pequenas. [RC1](evidence/rc1-bench.csv) · [Candidato](evidence/candidate-bench.csv).
- [Matriz do EnginePort](evidence/timing.csv), 108 condições, 44,1/48/96 kHz × 1/2/4x × 1/6/12 notas × quatro topologias, preset inicial e rotas Sum, 480 frames. Por condição, 16 blocos de aquecimento e 128 cronometrados. Todos produziram PCM finito. O contador observou zero chamadas alloc/free durante process, incluindo o início das notas. A alocação líquida retida na preparação foi 1.858.976 bytes em cada condição. O contador abrange o alocador Rust do processo de teste.
- **Gate de zero perdas FAILED.** No conjunto foram 34 perdas em 13.824 blocos cronometrados, distribuídas por 15 condições. Com Vector4/12 vozes/4x a 96 kHz, p50/p95/p99/máximo = 3,318/4,903/6,533/7,667 ms, 5 perdas em 128 blocos de 5 ms. Na mesma configuração a 48 kHz o máximo foi 6,699 ms para um orçamento de 10 ms. Os tempos são de processamento offline. Consumo térmico, jitter do dispositivo, XRuns e latência acústica continuam OPEN.

## Medições de sinal

- [Glide e transições de Unison](evidence/unison.csv). O teste das duas leis de glide compara o pitch a 50 ms com o valor previsto e desativa o glide no meio da nota. Para a troca 1→12, 12→2 ou 6→6 com mudança de detune/spread sob sustain, um segundo motor reproduz a mesma nota sem edição. A maior diferença entre motores no primeiro frame da edição foi 0,00025405 FS. O CSV separa esse erro do delta natural da forma de onda; não estabelece um limiar perceptivo.
- [Drive](evidence/drive.csv), 324 linhas. Medição direta do VCF com 8.192 frames de acomodação e 8.192 de análise, entrada coerente e decimador de voz a 1/2/4x. `tones=1` usa o bin 64; `tones=2` soma 64+107; `tones=3` usa 1703 com amplitude 0,8 e cutoff 0,4×Fs. Nos dois primeiros o cutoff é 2 kHz e a amplitude é 0,25 por tom. Ressonância 0,2. O CSV inclui RMS, DC, pico de amostra, estimativa de pico reconstruído a 4x, THD dos harmônicos 2–8, dois produtos IMD3 e a componente dobrada do 3º harmônico do tom alto, conforme a excitação. A/B de compensação aplica o ganho pós-VCF ao mesmo sinal capturado. Em Curtis a 48 kHz, Drive 4, o componente dobrado mediu −13,58/−111,46/−105,55 dBc a 1/2/4x. A medição de um bin não representa toda a energia de alias. Com o seno grave nesse mesmo filtro a 1x, o pico reconstruído passou de 0,107 para 0,427 FS com compensação. O ganho integral do Drive é opcional.
- [VCF em 27,51 Hz, res 1, drive 1,1, morph 0,35](evidence/vcf.csv). Três taxas, 4.096 e 65.536 amostras, zero/impulso/impulso quadruplicado em instâncias independentes. Todas as seis sondagens retornaram LongTail e zero pontos da curva. Testes com ganho linear conhecido, DC autônomo, sinal alternante, ganho dependente de amplitude e não finitos cobrem o classificador.
- [LegacyRC1 e DiffuseV1](evidence/reverb.csv). Impulso 0,5 à esquerda, saída wet, 6 s, Forte, shimmer 0,7 e decay nominal de 1,5 s. EDT/T20/T30 são tempos extrapolados a −60 dB pela curva de Schroeder. Em 48 kHz o T20 extrapolado passou de 1,287 para 1,334 s e a clareza C80 de −7,70 para −7,84 dB. O arquivo inclui pico, correlação estéreo e RMS do último quarto de segundo. O ensaio de freeze/kill é separado. [Amostra A](evidence/listening/A.wav) e [amostra B](evidence/listening/B.wav) contêm a mesma frase, ajustadas para −23 LUFS; [chave de identificação](evidence/listening/key.json) para abrir após a escuta. Preferência humana OPEN.

## Reprodução

Na raiz do worktree A, em Bash, com rustc e Cargo 1.100.0, NumPy, SciPy e FFmpeg disponíveis:

```bash
set -eu
out=$(mktemp -d)
probe=tests/dsp-qualification/Cargo.toml
cargo test --release --offline --locked -p prophet-control -p prophet-dsp -p prophet-params -p prophet-render -p prophet-analysis-core
cargo test --release --offline --locked --manifest-path crates/prophet-preset/Cargo.toml
cargo check --offline --locked --workspace
cargo clippy --offline --locked -p prophet-control -p prophet-dsp -p prophet-params -p prophet-analysis-core --all-targets
for t in x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu aarch64-linux-android; do
  cargo build --offline --locked --manifest-path crates/prophet-dsp-core/Cargo.toml --target "$t"
done
cargo run --release --offline --locked --manifest-path "$probe" --bin signals -- "$out/signals"
cargo run --release --offline --locked --manifest-path "$probe" --bin audition -- "$out/signals"
manifest="$PWD/docs/wo-a/evidence/signal-inputs.sha256"
(cd "$out/signals" && sha256sum --check --quiet "$manifest")
python3 tests/dsp-qualification/measure.py "$out/signals" "$out/csv"
python3 tests/dsp-qualification/listen.py "$out/signals" "$out/listening"
cargo run --release --offline --locked --manifest-path "$probe" --bin seams > "$out/unison.csv"
cargo run --release --offline --locked --manifest-path "$probe" --bin timing > "$out/timing.csv"
cargo run --release --offline --locked -p prophet-render --bin prophet-render -- --demo-all "$out/a-wav" --sample-rate 48000 --seconds 1
cargo run --release --offline --locked -p prophet-render --bin prophet-bench > "$out/a-bench.csv"
git worktree add --detach "$out/rc1" 2978fdc02c85aa413dc76cc468cebb05f5cecc2a
(cd "$out/rc1" && cargo run --release --offline --locked -p prophet-render --bin prophet-render -- --demo-all "$out/rc1-wav" --sample-rate 48000 --seconds 1)
(cd "$out/rc1" && cargo run --release --offline --locked -p prophet-render --bin prophet-bench > "$out/rc1-bench.csv")
diff -qr "$out/rc1-wav" "$out/a-wav"
```

O script de escuta embaralha A/B a cada execução. O manifesto de sinais cobre os 170 arquivos f32 antes do embaralhamento. Compare tempos sob a mesma carga e execute os benchmarks em série.

## Histórico do checkpoint 9fb6825

Ambiente Linux x86_64 com rustc 1.100.0 (`0ef1bc386`) e Cargo 1.100.0 (`3acc79c0c`). RC1 `2978fdc02c85aa413dc76cc468cebb05f5cecc2a`. Candidato `f186ea10d752092d1d0b8ebc7c44226398f7d124`.

## Regressões e cross-build

```bash
cargo test --locked -p prophet-control -p prophet-dsp -p prophet-params -p prophet-analysis-core -p prophet-render
cargo test --locked --release -p prophet-dsp --test reverb_regression
cargo clippy --locked -p prophet-control -p prophet-dsp -p prophet-params -p prophet-analysis-core --all-targets
rustup target add aarch64-unknown-linux-gnu aarch64-linux-android
for target in x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu aarch64-linux-android; do
  cargo build --locked --manifest-path crates/prophet-dsp-core/Cargo.toml --target "$target"
done
```

RC1 passou 37 testes nos quatro crates de núcleo antes das alterações. O teste de Unison em `03a58bf` reproduziu a regressão com 0 gates ativos quando 5 eram esperados. O candidato passou esse teste. No candidato, 42 testes dos cinco crates passaram, incluindo o render. Clippy encerrou com código 0 e avisos. Os 13 testes release de reverb passaram. O rlib sem std compilou para os três alvos. Consulte os resumos em `evidence/`.

## Comparação dos WAVs

Execute separadamente na RC1 e no candidato, em checkouts limpos. Use diretórios de saída distintos.

```bash
cargo run --locked --release -p prophet-render --bin prophet-render -- \
  --demo-all /tmp/prophet-demo --sample-rate 48000 --seconds 1
(cd /tmp/prophet-demo && sha256sum *.wav)
```

Os 11 WAVs estéreo de 1 s a 48 kHz têm SHA-256 idêntico entre os dois commits, incluindo Classic2, doze vozes, Unison, Curtis, Moog, chorus, hard sync e LegacyRC1 Ensemble. Os hashes estão em [evidence/wav-parity.csv](evidence/wav-parity.csv). A comparação exercita a API de render legada. A paridade de DiffuseV1, o flag de saída FloatBus, outros rates e recalls/automação dos hosts requerem testes específicos.

## Desempenho ainda aberto

Este checkpoint registra paridade por render e regressão funcional. Não há novo conjunto p50/p95/p99/máximo, alocações ou deadline. Para retomar, rode `cargo run --locked --release -p prophet-render --bin prophet-bench` na RC1 e no candidato com a mesma máquina e carga. A ferramenta existente não produz p95. Complete uma matriz A-owned de 44,1/48/96 kHz, 1/6/12 vozes, 1x/2x/4x, buffers incluindo 480, filtro Curtis/Moog, FX secos e ativos. Colete p50/p95/p99/máximo, contagem de misses, tamanho/heap e alocação em callback. Registre revisão, CPU, carga, duração e hashes. Tempo de bloco offline não é latência acústica.
