# Evidência reproduzível do checkpoint

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
