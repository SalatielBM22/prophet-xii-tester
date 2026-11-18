# Retomada do Agente A

A retomada de 9fb6825 foi executada no branch A. O novo ponto de implementação é `1f655c92f60a6a6124e7459014447f51f7d21517`. Leia [CONTRACT_HANDOFF.md](CONTRACT_HANDOFF.md) e a matriz atual antes de iniciar outro ciclo. Preserve commits posteriores a essa revisão.

## Registro histórico da migração


O proprietário relatou em 2026-11-17 que não conseguia alterar as configurações do ambiente remoto Codex Cloud. A ação exata na interface, a mensagem de erro e a causa não foram informadas. Durante o checkpoint, uma tentativa de usar o executor retornou “No usable execution environment is available”. O executor voltou a responder e o worktree reapareceu com as alterações intactas. A relação entre esses dois sintomas permanece OPEN. Nenhuma configuração da nuvem foi alterada nesta retomada.

A publicação deste branch no GitHub é o ponto de recuperação do trabalho. O código e os relatórios podem ser recuperados sem o worktree original, sem o cache de Cargo e sem os arquivos temporários. O ambiente original tinha fontes em `/workspace/prophet-dual-core` e ferramentas em `/workspace/.prophet-tools`. Esses caminhos são opcionais para a reprodução.

## Recuperar a fonte

Configure o novo ambiente com o fork `SalatielBM22/prophet-xii-tester`. Para um checkout novo:

```bash
git clone --single-branch --branch feat/dual-xhigh-core-dsp-vector \
  https://github.com/SalatielBM22/prophet-xii-tester.git prophet-dual-core
cd prophet-dual-core
git remote -v
git status --short --branch
git log -6 --oneline
git merge-base --is-ancestor 2978fdc02c85aa413dc76cc468cebb05f5cecc2a HEAD
git ls-remote --heads origin refs/heads/feat/dual-xhigh-core-dsp-vector
```

Confirme que o SHA anunciado corresponde ao checkpoint indicado na conversa ou a um descendente autorizado. Use o branch A para continuar. O Gemini B possui `feat/dual-xhigh-host-ui-platform` e coordena mudanças compartilhadas na futura integração `integration/dual-xhigh-rc2-rc3`. Preserve a RC1, os presets, `main` e o upstream institucional.

## Restaurar ferramentas e evidência

A verificação usou Rust e Cargo 1.100.0 em Linux x86_64. Ative uma toolchain equivalente. Cargo precisa acessar o registro crates.io e as dependências Git já pinadas em `Cargo.lock` para resolver o workspace. Se o acesso falhar, registre a operação e o erro. A biblioteca independente sem dependências externas pode ser verificada com `cargo build --locked --offline --manifest-path crates/prophet-dsp-core/Cargo.toml`. Consulte [BENCHMARKS.md](BENCHMARKS.md) para comandos de testes, render e comparação com RC1. Drivers de áudio e SDKs de plataformas pertencem aos testes de host do B.

A ordem original está no anexo `PROPHET-XII_Dual_Gemini_XHigh_Reference_Pack.zip`, SHA-256 `c156f7236610bac4bd6de10e256e8192f7b2b99857162a41410d626b6c4bd6ac`. Se o anexo estiver disponível no novo chat, verifique `SHA256SUMS.txt` após extrair e leia `prompts/WO_A_GEMINI_XHIGH.md`, `architecture/`, `planning/` e `references/`. [IMPLEMENTATION_MATRIX.md](IMPLEMENTATION_MATRIX.md) registra as frentes, os próximos passos e as fronteiras de propriedade. O pacote fornece bibliografia comentada. Os PDFs/livros externos não estavam incluídos. O proxy respondeu 403 às tentativas de consultar o manual Prophet VS e as referências FDN nesta sessão. A qualificação dependente das fontes primárias continua OPEN.

## Onde continuar

1. Leia o [handoff do Gemini B](CONTRACT_HANDOFF.md) e preserve o contrato no SHA `995d772d7371480900fe74def6bb3c60674abddc`. Verifique se B registrou aceite ou solicitação de mudança.
2. Retome G1/G3 pelos testes pendentes e pelas lacunas observáveis no [relatório](FINAL_AUDIT.md). Trate as escolhas `FloatBus` e `DiffuseV1` como mecanismos em qualificação. Avance o `no_std` por módulos com paridade.
3. Implemente G4 e G5 de acordo com a matriz. Mantenha as novas topologias experimentais e Classic2 como referência. O clock de música e a persistência pertencem ao núcleo Rust.
4. Qualifique G6 em comparação com RC1. Publique commits adicionais somente no mesmo branch A do fork. Códigos de hosts, Android, React e artefatos de dispositivos pertencem ao B.
