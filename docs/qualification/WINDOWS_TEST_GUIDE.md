# Guia de Instalação e Teste no Windows (RC1)

> [!IMPORTANT]
> Este pacote foi compilado de forma cruzada no Linux (`x86_64-pc-windows-msvc` com CRT estático `+crt-static`) e verificado quanto a cabeçalhos PE/COFF, importações/exportações e ausência de dependências externas de runtime (`VCRUNTIME140.dll`, `api-ms-win-crt-*.dll`). **Status de dispositivo real: `DEVICE_VERIFIED: false`** — a execução com áudio WASAPI real, MIDI físico e hosts DAW Windows ainda não foi realizada e deve ser validada localmente pelo usuário.

## 1. Pacote Entregue e Integridade

- Arquivo: `PROPHET-XII-RC1-Windows-x64.zip` (`2.913.793` bytes, `27` arquivos)
- Checksum SHA-256 (`PROPHET-XII-RC1-Windows-x64.zip.sha256`):
  `ff22d6cc7565e866578683575bbd34b68426c06a98aded49e92fa5c01332e4b8`
- Commit de código-fonte: `e23fa95be5f5cc8f8f082ae2849b7cc210834ac2` (branch `release/prophet-xii-rc1-qualification`)
- Licença: GNU GPL v3.0 ([`LICENSE`](file:///home/antigravity/SalatielBM22/prophet-xii-tester/LICENSE), copiado como `COPYING` dentro do pacote)

Para conferir a integridade do ZIP no Windows PowerShell antes de extrair:

```powershell
Get-FileHash .\PROPHET-XII-RC1-Windows-x64.zip -Algorithm SHA256
# Esperado: FF22D6CC7565E866578683575BBD34B68426C06A98ADED49E92FA5C01332E4B8
```

## 2. Conteúdo do Pacote

```text
PROPHET-XII-RC1-Windows-x64/
├── Start-PROPHET-XII.bat                     # Atalho de clique duplo (WASAPI + navegador)
├── Start-PROPHET-XII.ps1                     # Launcher PowerShell parametrizável
├── README-WINDOWS.txt                        # Notas rápidas e solução de problemas
├── BUILD-INFO.json                           # Metadados exatos de build, hashes e provenance
├── SHA256SUMS.txt                            # Checksums dos 26 arquivos restantes do pacote
├── COPYING                                   # GNU GPL v3.0
├── bin/
│   ├── prophet-standalone.exe                # Host standalone WASAPI + servidor HTTP/WS + UI
│   ├── prophet-render.exe                    # Renderizador determinístico offline (.wav / .mid)
│   ├── prophet-presets.exe                   # Inspetor dos 10 factory presets
│   └── prophet-probes.exe                    # Ferramenta de sondagem DSP offline
├── plugins/
│   ├── PROPHET-XII.clap                      # Plugin CLAP Windows x64 (não validado em DAW)
│   └── PROPHET-XII.vst3/
│       └── Contents/x86_64-win/
│           └── PROPHET-XII.vst3              # Bundle VST3 Windows x64 (não validado em DAW)
├── presets/
│   └── 00-factory.json .. 09-factory.json    # Os 10 presets originais de fábrica
└── ui/
    ├── index.html                            # UI React compilada (Vite production build)
    └── assets/                               # Bundle JS, CSS, Spectrum Worker e prophet_analysis_bg.wasm
```

## 3. Instalação Reversível e Pré-requisitos

- **Sem instalador e sem Visual C++ Redistributable**: todos os 5 binários PE foram ligados estaticamente ao CRT do Windows (`+crt-static`). Nenhuma chave de registro ou serviço do sistema é criada.
- **Reversibilidade**: basta encerrar o aplicativo e apagar a pasta extraída. Caso copie manualmente os plugins para `Common Files`, basta apagar os dois arquivos adicionados.
- **Passos de extração**:
  1. Extraia o arquivo `PROPHET-XII-RC1-Windows-x64.zip` para uma pasta de usuário (ex.: `C:\Users\<seu-usuario>\prophet-xii-rc1\`), preservando todas as subpastas.
  2. Se o Windows bloquear arquivos baixados da internet, opcionalmente execute `Unblock-File -Path C:\...\prophet-xii-rc1\*` no PowerShell, ou siga a instrução do SmartScreen abaixo.

## 4. Executando o Standalone (WASAPI + Interface Web)

### Modo clique duplo

1. Dê duplo clique em `Start-PROPHET-XII.bat`.
2. **SmartScreen**: como o RC1 não possui assinatura Authenticode comercial, o Windows Defender SmartScreen pode exibir *"O Windows protegeu o computador"*. Clique em **Mais informações** → **Executar mesmo assim**.
3. O terminal imprimirá a configuração de áudio WASAPI detectada e abrirá automaticamente `http://127.0.0.1:8787` no navegador padrão (implementado em [`main.rs`](file:///home/antigravity/SalatielBM22/prophet-xii-tester/crates/prophet-standalone/src/main.rs#L377-L391)).
4. Para encerrar com segurança (enviando All Notes Off), pressione `Ctrl+C` na janela do terminal ou feche a janela.

### Modo diagnóstico e parâmetros via PowerShell / CMD

1. Listar dispositivos WASAPI e portas MIDI detectadas pelo sistema:
   ```powershell
   .\bin\prophet-standalone.exe --list-devices
   # ou:
   .\Start-PROPHET-XII.ps1 -ListDevices
   ```
2. Forçar dispositivo específico, taxa de amostragem (`48000` ou `96000`) e tamanho de buffer:
   ```powershell
   .\Start-PROPHET-XII.ps1 -Device "Alto-falantes (Realtek(R) Audio)" -SampleRate 48000 -BufferFrames 256
   ```
3. Se a porta padrão `8787` estiver ocupada:
   ```powershell
   .\Start-PROPHET-XII.ps1 -Bind "127.0.0.1:8788"
   ```
4. Testar sem placa de áudio (clock de software `--null`):
   ```powershell
   .\bin\prophet-standalone.exe --null --ui ui --duration 5
   ```

> [!NOTE]
> Esta versão utiliza **WASAPI compartilhado** via CPAL; ASIO não está compilado. Para taxas como `96000 Hz`, o dispositivo selecionado no Painel de Controle de Som do Windows precisa suportar o formato correspondente, ou `prophet-standalone.exe` reportará erro claro de configuração não suportada.

## 5. Verificações Offline (WAV, Presets e Probes)

Antes mesmo de testar áudio em tempo real, você pode validar a execução local dos binários offline em PowerShell:

```powershell
# 1. Conferir hashes SHA-256 internos do pacote:
Get-Content .\SHA256SUMS.txt | ForEach-Object {
  $exp, $rel = $_ -split '  ', 2
  $act = (Get-FileHash (Join-Path . $rel) -Algorithm SHA256).Hash.ToLower()
  if ($act -ne $exp) { Write-Error "DIVERGENTE: $rel" }
}
"Todos os arquivos OK"

# 2. Listar presets e executar probes DSP:
.\bin\prophet-presets.exe
.\bin\prophet-probes.exe

# 3. Renderizar um WAV de referência a 48 kHz e 96 kHz:
.\bin\prophet-render.exe --demo twelve-voices --sample-rate 48000 --seconds 4 --output .\twelve-48k.wav
.\bin\prophet-render.exe --demo reverb-ensemble --sample-rate 96000 --seconds 5 --output .\reverb-96k.wav
```

## 6. Teste Manual dos Plugins CLAP e VST3 (Opcional)

O binário `prophet_plugin.dll` empacotado em `plugins\PROPHET-XII.clap` e `plugins\PROPHET-XII.vst3\Contents\x86_64-win\PROPHET-XII.vst3` exporta `clap_entry`, `GetPluginFactory`, `InitDll` e `ExitDll` (confirmado via `llvm-readobj --coff-exports`), mas **não foi carregado em nenhuma DAW Windows real**.

Para testar manualmente em uma DAW compatível (ex.: REAPER, Bitwig Studio, Ableton Live 12 / Cubase para VST3):

1. Copie `plugins\PROPHET-XII.clap` para `C:\Program Files\Common Files\CLAP\` e/ou a pasta `plugins\PROPHET-XII.vst3` para `C:\Program Files\Common Files\VST3\`.
2. Faça re-scan de plugins na DAW.
3. Carregue o instrumento em uma trilha MIDI, teste notas polifônicas, automação de parâmetros (`p0`–`p82`, além de `Reverb Enabled` e `Reverb Kill Tail`), e salve/reabra o projeto para verificar persistência de estado na DAW.
4. Para reverter completamente, feche a DAW e remova os arquivos copiados de `Common Files`.

## 7. Roteiro de Validação e Retorno (`DEVICE_VERIFIED`)

Ao testar no seu computador Windows, por favor observe e registre:

- [ ] **Inicialização e SmartScreen**: `Start-PROPHET-XII.bat` abre console e navegador sem erro de DLL ausente (`VCRUNTIME140.dll`).
- [ ] **WASAPI e Listagem**: `prophet-standalone.exe --list-devices` lista sua placa de áudio e portas MIDI sem falhas.
- [ ] **Áudio em Tempo Real**: Ao tocar no teclado virtual (C2–C7) ou MIDI externo, o áudio sai limpo sem estalos, XRuns/underruns (`device errors = 0`) ou travamentos em `48 kHz` (`256` e `512` frames) e, se suportado pelo driver, `96 kHz`.
- [ ] **Telemetria e Controles UI**: Presets 0–9 carregam; duplo-clique no knob reseta para o valor do preset; `Undo`/`Redo`; envelopes ADSR por voz animam estágios reais; Reverb ON/BYPASSED + `Spill`/`Kill` respondem sem piscar o botão.
- [ ] **Análise WASM e VCF**: Espectro ao vivo responde ao som com janela/tamanho FFT alternáveis (Hann / Blackman-Harris / Kaiser; 2048/4096/8192); gráfico de transferência do VCF exibe magnitude + fase na mesma taxa de amostragem do host ao mover `Cutoff`/`Resonance`/`Morph`.
- [ ] **Estabilidade sob carga**: Tocar acorde de 12 vozes com filtro `Moog` (`Morph = 1.0`) e `Oversampling = 4x` + `Ensemble = Forte` observando `DSP %` e `deadline misses` no cabeçalho da UI.
- [ ] **Plugins (se testados)**: DAW escolhida, versão do Windows, se CLAP e/ou VST3 foram reconhecidos, automatizados e restaurados após salvar projeto.
