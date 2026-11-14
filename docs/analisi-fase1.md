# Fase 1 — Analisi di lmu-pitwall e piano

Base: `Swizzjack/lmu-pitwall` v1.2.3 (commit `a171b80`), MIT. Codice: ~15 000 righe Rust e ~580 KB di TypeScript.

## Architettura attuale

```
PC del pilota (Windows, LMU aperto)
├── LMU_Data (shared memory)  ──┐   letta a 50 Hz (il gioco la scrive a 100 Hz)
├── REST :6397                ──┤   weather 5 s, strategy/usage 3 s, RepairAndRefuel 2 s
│                               ▼
└── lmu-pitwall.exe (Rust, tokio, un solo binario)
    ├── task_polling      legge la shared memory e la REST, gestisce lo stato
    ├── task_broadcaster  costruisce i messaggi e li manda in broadcast
    ├── fuel/             mediana mobile su 10 giri, conteggio stint, rilevamento pit
    ├── lap_tracker       snapshot per pilota a ogni passaggio sul traguardo
    ├── race_engineer/    callout vocali a REGOLE (Piper TTS); non è un LLM
    ├── fuel_calculator/  pianificatore multi-stint basato sullo storico
    ├── post_race/        SQLite: import degli XML dei risultati + live_laps per giro
    └── HTTP + WebSocket su 0.0.0.0:9000 (MessagePack, JSON con ?format=json)
           │
Browser (LAN) ── React 19 + zustand + react-grid-layout + recharts, embedded nel .exe
```

Build: cross-compilazione da Linux (`cargo zigbuild`), installer Inno Setup via Wine.
Ci sono 70 test Rust e il tool `tools/lmu-probe`, che verifica il layout della shared memory.

## Dati già esposti

- **Telemetria del giocatore** (~30 Hz via WS): velocità, giri motore, marcia, pedali, sterzo,
  carburante, temperature acqua e olio, gomme (temperature I/M/O, pressione, usura, freni),
  delta sul best.
- **Elettronica** (5 Hz): TC, ABS, mappa motore, barre antirollio, regen, brake migration.
- **Scoring di tutte le auto**: posizione, classe, giri, best/last, settori, gap,
  pit, posizione XYZ a 100 Hz, VE.
- **Sessione** (1 Hz): pista, meteo attuale, previsione a 5 nodi.
- **Stato vettura**: danni aero, freni e sospensioni (REST).
- **Storico**: SQLite con le sessioni importate dagli XML e i giri live (fuel, VE e usura
  gomme del giocatore; VE degli avversari).

## Cosa manca rispetto agli obiettivi

| # | Obiettivo | Cosa c'è | Cosa manca |
|---|---|---|---|
| 2 | Remoto | Bind su `0.0.0.0`, quindi funziona già via IP Tailscale. Il frontend si riconnette con backoff 1→30 s | **Auto-shutdown dopo 45 s senza client**: in remoto spegne il bridge a metà gara. Nessuna autenticazione. `POST /api/shutdown` e `/api/set-port` (riavvia il processo) aperti a chiunque, CORS `*`. Comandi WS (installazione voci TTS) senza controllo. Backoff massimo di 30 s troppo lento; manca un heartbeat |
| 3 | Multi-pilota | — | Tutto. Ogni bridge conosce solo il proprio PC |
| 4 | Strategia | Mediana carburante su 10 giri, VE da REST, pianificatore offline | Trend, consumo VE dal vivo, degrado gomme, passo in calo, finestre pit, perdita in pit lane, what-if, crossover meteo |
| 5 | Timing | Classifica per classe, gap dal leader di classe, settori, mappa, bandiere, battle widget | Avviso per auto più veloci in arrivo, storico giri dei rivali diretti, log eventi (FCY, penalità, incidenti) |
| 6 | Storico | SQLite per giro | Stint per pilota, registrazione della telemetria, sessione unica su più PC |
| 7 | AI | Engine a regole con TTS | Integrazione Claude, stato riassunto, rate limit e costi |
| 8 | Coaching | — | Tutto. Servono la telemetria a 100 Hz (oggi letta a 50 Hz) e la lap distance ad alta frequenza (è solo nello scoring a 5 Hz: va interpolata) |
| — | Replay/mock | — | Tutto |
| — | Tablet/PWA | Layout drag-and-drop | Manifest PWA e layout touch |

## Le tre verifiche richieste

Il gioco **non è installato** su questo PC: Steam ha il download in coda
(`SizeOnDisk 0`, 48 GB da scaricare). Le risposte qui sotto vengono quindi dal codice
upstream, che è stato misurato su LMU 1.4, e non da test miei.

1. **Altri piloti: solo scoring o anche telemetria?** Anche telemetria, ma parziale.
   Ogni auto ha la sua riga di telemetria (104 slot). Sono reali la posizione a 100 Hz e
   `mVirtualEnergy`. `mFuel` (fisso a metà serbatoio) e `mWear` (fisso a 1.0) sono
   segnaposto. Pedali, temperature gomme e freni degli altri: **da verificare**
   (`lmu-probe` run C, offline e online).
2. **Da spettatore/engineer vedo la telemetria della macchina del team?** Quasi
   certamente **no**. La macchina del team sarebbe un'auto "non nostra", quindi carburante
   e usura avrebbero i segnaposto del punto 1. Inoltre `playerHasVehicle` sarà 0. Il bridge
   sui PC dei piloti resta necessario per carburante, gomme, elettronica e coaching. Da
   confermare entrando come spettatore in una sessione.
3. **Meteo e VE dove?** Il meteo attuale è nella shared memory (scoring); la previsione
   (5 nodi) è solo REST `/rest/sessions/weather`. La VE dal vivo è nella shared memory
   (`mVirtualEnergy`, per tutte le auto); lo storico VE per giro è su REST
   `/rest/strategy/usage`; la capacità VE su REST `RepairAndRefuel.fuelInfo`.
   Dettagli in [lmu-api.md](lmu-api.md).

## Proposta per le fasi 2–3

**Hub sul tuo PC.** I bridge restano sui PC dei piloti, ognuno con il proprio token.
Un processo "hub" (lo stesso .exe, in modalità `--hub`) gira da te e:

- si collega via Tailscale a tutti i bridge del team;
- sceglie come fonte attiva il bridge in cui l'auto del team ha `mControl == 0`
  (guidata in locale) e `playerHasVehicle == 1`;
- unisce i dati in un'unica sessione e la salva in SQLite;
- serve la dashboard al tuo browser o tablet, e più avanti farà girare
  strategia, coaching e AI.

Vantaggi: storico e chiave Anthropic restano solo da te; i PC degli amici fanno solo
da sorgente; un cambio pilota non interrompe la sessione.

Modifiche al bridge già previste per la fase 2:

- auto-shutdown disattivato quando il token è configurato;
- token richiesto su WS e HTTP, e `/api/shutdown` / `set-port` solo da localhost;
- heartbeat ping/pong, backoff massimo di 5 s e snapshot completo alla riconnessione.

## Prerequisiti mancanti su questo PC

- **LMU**: completare il download in Steam (≈48 GB) e attivare
  *Settings → Gameplay → Enable Plugins*.
- **Rust** (rustup, target MSVC) e **Node.js LTS**: assenti. La build upstream è pensata
  per Linux; qui compilerò nativamente per Windows.
- **Fork su GitHub**: `gh` non è installato. Il remote `origin` punta già a
  `github.com/pixelgoat8/lmu-pitwall`: basta cliccare *Fork* su GitHub (oppure installare
  `gh` e fare login) prima del primo push.
