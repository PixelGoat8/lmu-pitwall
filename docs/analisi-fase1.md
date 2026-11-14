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

Misurate il 2026-11-14 su LMU 1.4.2, in una sessione offline di prove a Monza
(37 auto, Hypercar + LMP2 + GTE, 3 giri guidati). Dettagli e campioni in
[lmu-api.md](lmu-api.md).

1. **Altri piloti: solo scoring o anche telemetria?** Anche telemetria, e
   offline contro l'AI è molto ricca:
   - reali: posizione, velocità, **pedali, sterzo, marcia, giri motore**,
     **carburante**, **usura gomme**, **temperatura freni**, e VE e batteria
     per le Hypercar;
   - non pubblicati (sempre 0): temperature e pressioni gomme, elettronica
     (TC, ABS, mappa).

   **Attenzione:** l'autore upstream ha misurato online carburante fisso a metà
   serbatoio e usura fissa a 1.0 per le altre auto. Online il gioco pubblica
   quindi meno dati. Va rimisurato in una sessione online.
2. **Da spettatore/engineer vedo la telemetria della macchina del team?**
   **Non ancora verificato**: serve una sessione online. Due indizi:
   - offline la telemetria delle altre auto c'è, quindi non è escluso;
   - online upstream ha visto valori fissi per carburante e usura delle altre auto.

   Finché non è misurato, il progetto assume che serva il bridge sul PC di chi guida.
3. **Meteo e VE dove?**
   - Meteo attuale: shared memory (scoring), e anche REST `RepairAndRefuel.currentWeather`.
   - Previsione (5 nodi): solo REST, `/rest/sessions/weather`.
   - VE dal vivo: shared memory `mVirtualEnergy` (tutte le auto con VE).
   - Storico per giro: REST `/rest/strategy/usage`, per **tutti** i piloti
     (VE, stint, pit); per la propria auto anche carburante e usura gomme.

### Scoperte utili per le fasi successive

- **`RepairAndRefuel` contiene il modello della sosta**: velocità di
  rifornimento carburante e VE, tempi di cambio gomme (12 s per quattro) e
  cambio pilota (25 s), cosa avviene in parallelo. `/rest/strategy/pitstop-estimate`
  dà il tempo da fermo della sosta pianificata, voce per voce. La strategia
  (fase 4) può usare questi numeri del gioco; resta da misurare solo il tempo
  di percorrenza della pit lane per pista.
- **`teamInfo.driverNames`** elenca i piloti dell'auto: utile per il multi-pilota.
- **Dati fantasma dopo l'uscita dalla sessione**: tornati al menu, la shared
  memory conserva l'ultimo frame (37 auto, `playerHasVehicle` = 1). L'unico
  segnale è l'evento `SME_UNLOAD` e il clock fermo. Il bridge upstream non
  guarda gli eventi: va corretto, altrimenti l'hub crederebbe che un pilota
  sia ancora in pista.
- **Segnale "nuovo frame" e lock** nell'header 1.4.2: permettono di leggere
  tutti i frame a 100 Hz senza copie incoerenti (serve al coaching).
- Il layout della shared memory coincide con gli header ufficiali; i 90 campi
  letti dal bridge sono tutti plausibili; 0 letture incoerenti su 4207.

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


## Ambiente di sviluppo su questo PC

Installati con winget: Rust 1.100 (toolchain MSVC), Node.js 26, Visual Studio
Build Tools 2022 (C++). La build upstream è pensata per Linux
(`cargo zigbuild`); qui frontend e bridge compilano in nativo su Windows e i
70 test del bridge passano.

## Ancora da verificare con il gioco

1. Sessione online: dati delle altre auto (carburante, usura, pedali).
2. Spettatore/engineer: telemetria e REST dell'auto del team.
3. Cambio pilota: `mControl`, `mIsPlayer` e chiave di `/rest/strategy/usage`.
4. Pioggia e previsione con meteo accelerato.
