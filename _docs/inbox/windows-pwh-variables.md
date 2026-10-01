# Ambiente della sessione Claude Code: variabili e cartelle

Rilevato il 2026-10-01 dal processo PowerShell avviato da Claude Code su questa macchina
(utente `marco.t`, repo `D:\Sviluppo\mcps\po-mcp`, .NET SDK 10.0.401).

## Sintomo

In questa sessione `dotnet test` / `dotnet build` non terminano:

- il restore NuGet fallisce con `NuGet.targets(782,5): error : Value cannot be null. (Parameter 'path1')`;
- impostando alcune variabili a mano il restore passa, ma la build resta ferma: MSBuild avvia circa
  30 nodi `dotnet MSBuild.dll /nodemode:1` che restano inattivi (CPU ~0,5 s) finché non scade il timeout;
- nella radice del repo compare una cartella letterale `%SystemDrive%\ProgramData\Microsoft\Windows\Caches\*.db`
  (variabile non espansa; è un artefatto, va cancellata e non va committata);
- anche `git` lanciato dal tool Bash è andato in timeout (da PowerShell funziona).

## Causa probabile

La sessione gira in un ambiente isolato: sono presenti `ISOL8_SANDBOXED` e `ISOL8_PATH_POLICY`.
Il wrapper passa al processo solo una lista ridotta di variabili e limita le cartelle scrivibili.
Nel registro (Machine/User) non c'è nulla da correggere: queste variabili le imposta Windows a ogni
avvio sessione, non sono configurazione persistente.

## Variabili Windows mancanti

| Variabile | Valore normale |
|---|---|
| `SystemDrive` | `C:` |
| `windir` | `C:\Windows` |
| `ProgramData` | `C:\ProgramData` |
| `ALLUSERSPROFILE` | `C:\ProgramData` |
| `ProgramFiles` | `C:\Program Files` |
| `ProgramW6432` | `C:\Program Files` |
| `ProgramFiles(x86)` | `C:\Program Files (x86)` |
| `CommonProgramFiles` | `C:\Program Files\Common Files` |
| `CommonProgramFiles(x86)` | `C:\Program Files (x86)\Common Files` |
| `CommonProgramW6432` | `C:\Program Files\Common Files` |
| `PUBLIC` | `C:\Users\Public` |
| `COMPUTERNAME` | nome del PC |
| `USERDOMAIN` | dominio o nome del PC |
| `OS` | `Windows_NT` |
| `PROCESSOR_ARCHITECTURE` | `AMD64` |

Presenti: `SystemRoot`, `USERNAME`, `USERPROFILE`, `HOMEDRIVE`, `HOMEPATH`, `APPDATA`, `LOCALAPPDATA`,
`TEMP`, `TMP`, `ComSpec`, `PATHEXT`, `NUMBER_OF_PROCESSORS`.

Elenco completo dei nomi presenti nel processo: `AI_AGENT, APPDATA, CLAUDE_CODE_*, CLAUDECODE, COMSPEC,
COREPACK_ENABLE_AUTO_PIN, GCM_INTERACTIVE, GIT_ASKPASS, GIT_EDITOR, GIT_TERMINAL_PROMPT, HOME, HOMEDRIVE,
HOMEPATH, ISOL8_PATH_POLICY, ISOL8_SANDBOXED, LOCALAPPDATA, NO_COLOR, NoDefaultCurrentDirectoryInExePath,
PATH, PATHEXT, PROMPT, PSExecutionPolicyPreference, PSModulePath, PYTHONIOENCODING, SYSTEMROOT, TEMP, TMP,
USERNAME, USERPROFILE`.

## Variabili .NET / NuGet / MSBuild

- Nessuna variabile `DOTNET_*`, `NUGET_*`, `MSBUILD*`, `ASPNETCORE_*` impostata (di per sé normale).
- `dotnet` si trova: `C:\Program Files\dotnet\dotnet.exe`; nel `PATH` ci sono `C:\Program Files\dotnet\`
  e `C:\Users\marco.t\.dotnet\tools`.
- Esistono `%APPDATA%\NuGet\NuGet.Config` e `%USERPROFILE%\.nuget\packages`.
- L'errore `path1 null` del restore deriva da `ProgramData` mancante (NuGet cerca la configurazione di macchina).

## Cartelle utili a .NET: scrivibili e no

Verifica fatta creando e cancellando un file sonda in ogni cartella.

| Cartella | Esito | Note |
|---|---|---|
| `D:\Sviluppo\mcps\po-mcp` (repo, `bin`/`obj`) | scrivibile | |
| `%TEMP%` (`C:\Users\marco.t\AppData\Local\Temp`) | scrivibile | |
| `C:\Users\marco.t\.nuget\packages` | scrivibile | cache dei pacchetti |
| `%APPDATA%\NuGet` | scrivibile | `NuGet.Config` utente |
| `%LOCALAPPDATA%\NuGet` (anche `v3-cache`) | scrivibile | |
| `C:\Users\marco.t\.dotnet` | scrivibile | |
| `C:\Users\marco.t\.dotnet\tools` | **NON scrivibile** | solo per `dotnet tool install -g` (es. `dotnet-ef`); usare i tool già installati va bene |
| `%APPDATA%\Microsoft\UserSecrets` | **NON scrivibile** | `dotnet user-secrets set` fallisce: per la stringa di connessione `ConnectionStrings:mcppo` usare variabile d'ambiente `ConnectionStrings__mcppo` o il file `.env` |
| `C:\ProgramData` | **NON scrivibile** | MSBuild/NuGet vi appoggiano cache e configurazione di macchina |
| `C:\ProgramData\Microsoft\NetFramework` | **NON scrivibile** | |
| `C:\Windows\Temp` | **NON scrivibile** | |
| `%LOCALAPPDATA%\Microsoft\VisualStudio` | **NON scrivibile** | non serve alla CLI |
| `C:\Program Files\dotnet` (e `sdk\10.0.401`) | **NON scrivibile** | normale, lo è anche fuori dal sandbox (richiede admin) |
| `%LOCALAPPDATA%\Microsoft\dotnet` | non esiste | |
| `%APPDATA%\ASP.NET\Https` | non esiste | il certificato di sviluppo sta nello store utente di Windows, non qui |
| `C:\ProgramData\NuGet` | non esiste | |

Quelle che contano per la build: `C:\ProgramData` e `C:\Windows\Temp` non scrivibili, più le variabili mancanti.
`.nuget\packages`, `%TEMP%` e il repo sono scrivibili, quindi il solo restore non è bloccato dai permessi.

## Cosa è stato provato

1. `dotnet test` da Bash e da PowerShell, senza modifiche: timeout, nodi MSBuild fermi.
2. Con `SystemDrive`, `ProgramData`, `ALLUSERSPROFILE`, `ProgramFiles`, `ProgramFiles(x86)` impostate solo per il
   processo: il restore passa, la build resta ferma.
3. Con l'intero elenco sopra impostato ma **senza** `-m:1`: il restore termina subito con exit code 1 e nessun
   messaggio, lasciando circa 20 nodi MSBuild orfani. Il parallelismo resta il problema.
4. Con l'intero elenco **e** `-m:1 -nodeReuse:false --disable-build-servers`: funziona. Restore in ~3 s,
   build e test completati, nessun processo residuo.
   Esito verificato il 2026-10-01: `PerPrincipalLifetimeSpikeTests` -> `Passed! Failed: 0, Passed: 1, Total: 1`.

## Soluzione: come lanciare dotnet in questa sessione

Servono entrambe le cose: le variabili dell'elenco sopra **e** MSBuild a un solo nodo.

```powershell
$env:SystemDrive='C:'; $env:windir='C:\Windows'; $env:ProgramData='C:\ProgramData'; $env:ALLUSERSPROFILE='C:\ProgramData'
$env:ProgramFiles='C:\Program Files'; $env:ProgramW6432='C:\Program Files'; ${env:ProgramFiles(x86)}='C:\Program Files (x86)'
$env:CommonProgramFiles='C:\Program Files\Common Files'; $env:CommonProgramW6432='C:\Program Files\Common Files'
${env:CommonProgramFiles(x86)}='C:\Program Files (x86)\Common Files'
$env:PUBLIC='C:\Users\Public'; $env:COMPUTERNAME=[System.Net.Dns]::GetHostName().ToUpper(); $env:USERDOMAIN=$env:COMPUTERNAME
$env:OS='Windows_NT'; $env:PROCESSOR_ARCHITECTURE='AMD64'
dotnet test <progetto> -m:1 -nodeReuse:false --disable-build-servers
```

- Le variabili valgono solo per il processo PowerShell corrente; non si toccano registro né configurazione di sistema.
- Senza `-m:1` MSBuild avvia un nodo per CPU (qui 24) e la build non termina.
- Il tool Bash è inaffidabile in questa sessione (anche `git` va in timeout): usare PowerShell.
- Se compare di nuovo la cartella `%SystemDrive%\` nel repo, vuol dire che un comando è partito senza le variabili:
  cancellarla, non va committata.
