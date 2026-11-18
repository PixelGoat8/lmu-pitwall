# Contribuire a LMU Pitwall (remote engineer)

Benvenuto. Questo è il fork di [LMU Pitwall](https://github.com/Swizzjack/lmu-pitwall)
che usiamo come squadra: un programma sul PC di ogni pilota legge i dati di
Le Mans Ultimate, e l'ingegnere li segue da casa via Tailscale. Cosa fa, fase
per fase, è nel [README](README.md); come si usa, nelle guide in [`docs/`](docs).

Non serve saper programmare per contribuire: anche segnalare un problema
visto in gara o chiedere un dato che manca è un contributo (vedi
[Idee, dati e problemi](#idee-dati-e-problemi)).

## Come funziona, in breve

```
PC del pilota                         PC dell'ingegnere            Tablet / altro PC
┌──────────────────────────┐          ┌───────────────────┐        ┌─────────────┐
│ Le Mans Ultimate         │          │ Pitwall --hub     │        │  browser    │
│  ├ memoria condivisa ────┤          │  segue chi guida, │◄───────┤  (dashboard)│
│  └ API REST (:6397) ─────┤          │  strategia, AI,   │        └─────────────┘
│ Pitwall (bridge, :9000) ─┼─Tailscale─►  storico (:9100) │
│  dashboard nel browser   │          └───────────────────┘
└──────────────────────────┘
```

- **bridge** (`bridge/`, Rust): legge il gioco, calcola strategia, eventi,
  storico, commenti AI, e serve la dashboard e i dati via WebSocket.
  Con `--hub` invece del gioco legge i bridge dei piloti.
- **dashboard** (`dashboard/`, React + TypeScript): la pagina nel browser,
  fatta di widget. Viene compilata dentro l'`.exe` del bridge.

## Preparare il PC (Windows, una volta)

Apri PowerShell e installa gli strumenti con winget (se qualcuno l'hai già,
saltalo):

```powershell
winget install Git.Git GitHub.cli OpenJS.NodeJS.LTS Rustlang.Rustup Python.Python.3.12 Microsoft.VisualStudioCode
winget install Microsoft.VisualStudio.2022.BuildTools --override "--add Microsoft.VisualStudio.Workload.VCTools --includeRecommended --passive --wait"
```

Il secondo comando installa il compilatore C++ di Microsoft, che serve a Rust
su Windows: ci mette un po'. Poi **chiudi e riapri PowerShell** e collegati a
GitHub:

```powershell
gh auth login --web --git-protocol https
gh repo clone PixelGoat8/lmu-pitwall
cd lmu-pitwall
git checkout remote-engineer
```

Il lavoro della squadra è sul ramo **`remote-engineer`**, non su `main`
(che è il progetto originale).

## Compilare e provare

La dashboard va compilata **prima** del bridge, che la incorpora:

```powershell
cd dashboard; npm ci; npm run build; cd ..
cd bridge; cargo build; cd ..
```

La prima compilazione Rust dura qualche minuto, le successive pochi secondi.

### Senza il gioco: una gara finta

Non serve avere LMU aperto. `tools/mock-session` genera una sessione finta
(auto, pit stop, consumi, lift and coast…) e il bridge la riproduce come se
fosse il gioco:

```powershell
python tools/mock-session/make_session.py prova.pwrec --duration 900 --lap-time 30 --pit-every 8 --lico "6:3:0.08"
bridge\target\debug\lmu-pitwall.exe --replay prova.pwrec --ws-port 9001
```

Si apre il browser su `http://localhost:9001`. Le opzioni del generatore:
`python tools/mock-session/make_session.py --help` (team con cambio pilota,
classe GT3, full course yellow, penalità, bloccaggi, …).

Puoi anche **registrare una tua sessione vera** con `--record gara.pwrec` e
riprodurla quando vuoi. Attenzione: una registrazione vera contiene i nomi
dei piloti in pista, quindi non va messa nel repo (i `.pwrec` sono già esclusi).

### Lavorare sulla dashboard con il ricaricamento automatico

Con un bridge acceso (anche con `--replay`, come sopra):

```powershell
cd dashboard
npm run dev
```

Apri `http://localhost:5173` e, in **⚙ Settings**, imposta la porta del
bridge (per esempio 9001). Ogni modifica ai file in `dashboard/src` compare
subito nel browser.

### Con il gioco

`cargo build --release`, poi `bridge\target\release\lmu-pitwall.exe` con LMU
aperto e *Enable Plugins* attivo (vedi la [guida dei piloti](docs/guida-pilota.md)).

## Dove sono le cose

| Cartella | Cosa c'è |
|---|---|
| `bridge/src/main.rs` | avvio, lettura del gioco, invio dei messaggi |
| `bridge/src/shared_memory/` | lettura della memoria condivisa di LMU |
| `bridge/src/strategy/` | motore di strategia (`model.rs` sono i calcoli, puri e testati) |
| `bridge/src/hub/` | modalità hub: segue il PC di chi guida, stint |
| `bridge/src/race/` | eventi di gara e giri di tutte le auto |
| `bridge/src/ai/` | commenti AI (API Anthropic) e controllo dei numeri |
| `bridge/src/history/` | storico in SQLite |
| `bridge/src/protocol/messages.rs` | tutti i messaggi bridge ↔ dashboard |
| `dashboard/src/components/widgets/` | i widget |
| `dashboard/src/stores/telemetryStore.ts` | dove arrivano i messaggi del bridge |
| `dashboard/src/utils/` | calcoli lato dashboard (distacchi, gomme, mappa…) |
| `docs/` | guide per gli utenti (in italiano) e note tecniche |
| `tools/` | generatore di gare finte, sonde per le API del gioco |
| `installer/` | installer Windows (`build.ps1`) |

Alcune parti vengono dal progetto originale e potrebbero sparire nella fase
di pulizia (risultati post-gara, ingegnere vocale, fuel calculator…): prima
di lavorarci chiedi.

## Prima di proporre una modifica

1. **Crea un ramo** da `remote-engineer`: `git switch -c nome/cosa-fai`
   (es. `marco/gap-in-giri`).
2. **Commit piccoli**, ognuno con una cosa sola, messaggio in inglese che dice
   cosa cambia e perché.
3. **Controlla che tutto passi**:
   ```powershell
   cd bridge; cargo test; cd ..
   cd dashboard; npx tsc -b; npm run build; cd ..
   ```
   `npm run lint` segnala anche errori vecchi in file del progetto originale:
   basta non aggiungerne di nuovi (`npx eslint src/percorso/del/file.tsx`).
4. **Provalo** con una gara finta, e col gioco se tocca la lettura dei dati.
5. **Apri una pull request** verso `remote-engineer`:
   `git push -u origin nome/cosa-fai`, poi `gh pr create --base remote-engineer`.
   GitHub fa partire da solo compilazione e test; Leonardo la guarda e la unisce.

### Regole

- **Niente dati personali nel repo**: token, chiavi API, email, nomi veri
  presi da LMU (classifiche, risultati, registrazioni). Le schermate vanno
  ripulite.
- **La chiave Anthropic** si legge solo dalla variabile d'ambiente
  `ANTHROPIC_API_KEY`: mai nel codice, nei log o nei file di configurazione.
- **Nessun numero inventato**: ogni dato mostrato viene da una misura o dal
  gioco; se manca si mostra "—" e si spiega perché.
- **Lingue**: le guide per gli utenti in italiano; codice, commenti e
  messaggi di commit in inglese.
- Se cambi un messaggio tra bridge e dashboard, aggiorna entrambi i lati
  (`protocol/messages.rs` e `dashboard/src/types/`).

## Idee, dati e problemi

Usa le [Issue](https://github.com/PixelGoat8/lmu-pitwall/issues/new/choose)
del repo: ci sono due moduli, **Problema** (cosa è successo, in quale gara,
cosa ti aspettavi) e **Dato o idea** (cosa vorresti vedere, quando serve in
gara, chi lo guarda). Una schermata aiuta sempre.

## Cosa c'è in programma

1. Questo documento e l'organizzazione per i contributori.
2. Pulizia delle parti del progetto originale che non usiamo.
3. **Multi-team**: due squadre nella stessa gara (ufficiale e clienti), un hub
   per team, l'auto sorella evidenziata.
4. **Focus sui dati** che stiamo elencando insieme.
5. **Analisi delle telemetrie** registrate dal gioco (`UserData\Telemetry`):
   confronto giri, frenate, lift and coast.
6. **Grafica**.

## Rilasci

Li pubblica Leonardo: `python scripts/bump-version.py --minor` (o senza
opzioni per una correzione), `powershell -ExecutionPolicy Bypass -File
installer\build.ps1`, poi la release su GitHub con l'installer allegato.
Pitwall avvisa da solo i piloti quando c'è una versione nuova.
