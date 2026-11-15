# Motore di strategia

Il widget **Strategy** mostra il piano di gara ricalcolato a ogni giro
completato (e ogni 2 secondi tra un giro e l'altro). Lo calcola il bridge, o
l'hub se segui una squadra: in quel caso il piano continua attraverso i cambi
pilota. Si trova nel layout *Endurance* oppure da *+ Add widget → Strategy*.

Ogni numero è calcolato da giri misurati o da dati del gioco. Quando un dato
manca, il widget mostra "—" e in fondo spiega perché (per esempio "serve un
altro giro verde").

## Cosa mostra

| Sezione | Contenuto | Da dove viene |
|---|---|---|
| Intestazione | Giri ancora da fare, tempo rimasto, giro di riferimento, durata dello stint | Tempo/giri di sessione dal gioco; giro di riferimento = mediana degli ultimi giri puliti |
| Fuel / Virtual energy | Livello, consumo per giro e su quali giri è misurato, LiCo, giri rimasti, quanto serve per finire, quanto aggiungere, durata di un pieno, riserva, cosa serve per una sosta in meno, target | Livello letto a ogni passaggio sul traguardo; pedali e velocità dalla telemetria |
| Soste | Numero di soste e finestra (primo e ultimo giro utile), con la barra fino alla bandiera | Calcolo dal consumo, dalla durata del pieno e dal limite di stint |
| Box in … giri | Per "adesso" e per ogni giro della finestra: carburante e VE da aggiungere, soste che restano dopo, tempo da fermo, costo totale | Tempi di lavoro ai box del gioco + perdita in pit lane |
| Tyres | Giri sul treno attuale, gomma che si consuma di più, giri al limite di usura, calo di passo per giro, gomme nuove sì o no | Usura e tempi sul giro misurati |
| Pit lane | Perdita in pit lane, e la stima del gioco per il menu box come impostato ora | Misurata a ogni sosta, salvata per pista |
| Weather | Pioggia o asciutto, acqua in pista, quando è prevista pioggia (o il ritorno all'asciutto) in minuti e giri | Previsione a 5 punti del gioco |

## Come vengono fatti i conti

- **Consumo**: solo dai giri "verdi" consecutivi. Sono esclusi i giri passati
  dalla pit lane e quelli con bandiera gialla, perché consumano meno e
  renderebbero il piano ottimista. Il valore usato per pianificare è il consumo
  medio sugli ultimi 5 giri verdi dello stint: abbastanza per smorzare
  l'imprecisione della lettura sul traguardo, pochi per seguire un pilota che
  risparmia a poco a poco. Se il pilota cambia il lift and coast, conta solo dai
  giri fatti col LiCo nuovo (vedi sotto). Dopo una sosta, finché non ci sono
  due giri verdi, vale lo stint precedente. Accanto c'è il consumo dell'ultimo
  giro da solo.
- **Rispetto all'HUD del gioco**: la stima dei giri del gioco non è leggibile
  da Pitwall e pesa di più gli ultimissimi giri. Dopo un finale di stint in
  risparmio dà qualche giro in più del nostro; il primo giro del nuovo stint
  di solito torna a consumare come la media.
- **Carburante e usura** vengono letti solo dal PC che sta guidando: dagli
  altri PC il gioco mostra valori fissi.
- **Giri ancora da fare**: in una gara a tempo conta il giro in corso, i giri
  interi fino allo scadere e il giro che si sta facendo quando il tempo finisce.
  Usa il passo della tua auto, quindi se sei doppiato può essere un giro in più
  del reale (è il lato sicuro per il carburante).
- **Finestra box**: l'ultimo giro utile è quello in cui il serbatoio (o il
  limite di stint) finisce, riserva esclusa. Il primo è quello da cui le soste
  rimaste coprono tutta la gara con stint pieni.
- **Tempo da fermo**: calcolato dai tempi del gioco (velocità di rifornimento,
  cambio gomme, cambio pilota). Il gioco indica quali lavori sono
  "contemporanei": li faccio sovrapporre e sommo gli altri. È una mia
  interpretazione di quei flag, perciò accanto c'è la stima del gioco per il
  menu impostato, per confrontarle.
- **Perdita in pit lane**: misurata a ogni sosta come (tempi dei giri toccati
  dalla sosta) − (altrettanti giri di riferimento) − (tempo da fermo). Viene
  salvata per pista in `%APPDATA%\LMUPitwall\pit_loss.json` e si usa la mediana.
  Prima della prima sosta su una pista non c'è, a meno di inserirla a mano.
  Le soste fatte in prova (Practice, Test Day, Warmup) non vengono salvate: lì
  si rientra per cambiare assetto o si va in garage, e il dato sarebbe falsato.
- **Calo di passo con le gomme**: pendenza dei tempi sul giro nei giri puliti
  del treno attuale. Comprende anche l'auto che si alleggerisce consumando
  carburante, quindi il calo dovuto alle sole gomme è un po' maggiore.
- **Gomme nuove sì o no**: secondi in più da fermo contro secondi che le gomme
  nuove farebbero recuperare nello stint successivo, al calo di passo misurato.
  Se l'usura arriverebbe al limite durante lo stint successivo, il cambio è
  obbligato.
- **Meteo**: la previsione ha 5 punti per sessione (inizio, 25%, 50%, 75%,
  fine). Pioggia "probabile" vuol dire probabilità ≥ 50% o cielo da pioggia.
  Il momento in cui conviene passare alle gomme da bagnato non viene calcolato:
  servirebbero tempi sul giro misurati sul bagnato, che oggi non ci sono.

## Lift and coast (LiCo)

- **Misura**: a ogni giro, i secondi passati sopra i 72 km/h senza gas né freno
  (entrambi sotto il 5 %). Si legge dai pedali, che il gioco manda per
  qualsiasi auto: funziona anche dal posto engineer. Una parte di veleggio c'è
  sempre nella guida normale; quello che conta è come cambia. Il primo giro
  dopo l'avvio di Pitwall, i giri ai box e quelli con bandiera gialla non hanno
  il valore.
- **Il piano segue il LiCo**: quando il veleggio cambia di più di 1 s a giro per
  almeno due giri di fila, il consumo viene misurato solo dai giri col LiCo
  nuovo. Il widget scrive *since LiCo changed, N laps* e il LiCo di prima con il
  suo consumo. Così il piano si aggiorna dopo due giri, sia quando il LiCo
  aumenta sia quando il pilota smette. Un singolo giro diverso (traffico) non
  conta come cambio.
- **Quanto rende un secondo di LiCo**: dalla pendenza del consumo giro per giro
  rispetto al veleggio, sui giri verdi della sessione. Serve che il pilota
  abbia già veleggiato in modi diversi (almeno 6 giri, con almeno 1,5 s di
  differenza); prima i secondi di LiCo necessari non vengono stimati.
- **Una sosta in meno**: il consumo massimo per giro con cui basta una sosta in
  meno, il risparmio in percentuale e, se misurato, circa quanti secondi di LiCo
  in più servono. La riga compare solo se il risparmio è al massimo del 25 %.
- **Target**: nel campo *target* sotto Virtual energy (o Fuel) scrivi il
  consumo che chiedi al pilota, per esempio `3.2` per 3,2 %/giro, e premi
  *Set*. Il widget mostra di quanto è sopra o sotto, i secondi di LiCo
  necessari (o il margine) e, a quel consumo, giri rimasti, durata di un pieno
  e soste. Il piano principale resta sul consumo misurato. *Clear* toglie il
  target. Si può impostare anche dalle impostazioni, anche prima che ci sia un
  consumo misurato.

## Impostazioni (⚙ settings nel widget)

| Voce | Predefinito | Effetto |
|---|---|---|
| Reserve (laps) | 0,5 | Energia tenuta sempre nel serbatoio |
| Max stint (min) | nessuno | Durata massima di guida per pilota; può imporre soste che il serbatoio non richiede |
| Tyre limit (% tread) | 30 | Sotto questa percentuale di battistrada una gomma è finita |
| Pit lane loss (s) | misurata | Valore a mano al posto di quello misurato |
| New tyres at the stop | no | La tabella "Box in" include il cambio di 4 gomme |
| VE target (%/lap) | nessuno | Consumo di VE chiesto al pilota (vedi *Lift and coast*) |
| Fuel target (L/lap) | nessuno | Consumo di carburante chiesto al pilota |

Le impostazioni valgono per il bridge o l'hub a cui sei collegato e vengono
salvate in `%APPDATA%\LMUPitwall\strategy.json`.

## Limiti attuali

- Verificato solo con sessioni simulate (`tools/mock-session`), non ancora in
  una gara vera. Da controllare in pista:
  - che la perdita in pit lane misurata sia credibile;
  - che il tempo da fermo calcolato sia vicino alla stima del gioco;
  - che il conteggio dei giri a fine gara sia giusto.
- Le gare a giri usano i giri della tua auto, non quelli del leader.
- Il cambio di mappa motore non viene riconosciuto come il LiCo: il consumo si
  adatta nei giri successivi.
- Le soglie del LiCo (72 km/h, pedali sotto il 5 %, cambio di 1 s) sono scelte
  a tavolino e verificate solo con sessioni simulate (`--lico` in
  `tools/mock-session`): da controllare in gara.
