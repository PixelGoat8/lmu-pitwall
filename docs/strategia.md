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
| Fuel / Virtual energy | Livello, consumo per giro (mediana, con tendenza), giri rimasti, quanto serve per finire, quanto aggiungere, durata di un pieno, riserva | Livello letto a ogni passaggio sul traguardo |
| Soste | Numero di soste e finestra (primo e ultimo giro utile), con la barra fino alla bandiera | Calcolo dal consumo, dalla durata del pieno e dal limite di stint |
| Box in … giri | Per "adesso" e per ogni giro della finestra: carburante e VE da aggiungere, soste che restano dopo, tempo da fermo, costo totale | Tempi di lavoro ai box del gioco + perdita in pit lane |
| Tyres | Giri sul treno attuale, gomma che si consuma di più, giri al limite di usura, calo di passo per giro, gomme nuove sì o no | Usura e tempi sul giro misurati |
| Pit lane | Perdita in pit lane, e la stima del gioco per il menu box come impostato ora | Misurata a ogni sosta, salvata per pista |
| Weather | Pioggia o asciutto, acqua in pista, quando è prevista pioggia (o il ritorno all'asciutto) in minuti e giri | Previsione a 5 punti del gioco |

## Come vengono fatti i conti

- **Consumo**: solo dai giri "verdi" consecutivi. Sono esclusi i giri passati
  dalla pit lane e quelli con bandiera gialla, perché consumano meno e
  renderebbero il piano ottimista. Il valore usato per pianificare è la mediana
  degli ultimi 10 giri.
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

## Impostazioni (⚙ settings nel widget)

| Voce | Predefinito | Effetto |
|---|---|---|
| Reserve (laps) | 0,5 | Energia tenuta sempre nel serbatoio |
| Max stint (min) | nessuno | Durata massima di guida per pilota; può imporre soste che il serbatoio non richiede |
| Tyre limit (% tread) | 30 | Sotto questa percentuale di battistrada una gomma è finita |
| Pit lane loss (s) | misurata | Valore a mano al posto di quello misurato |
| New tyres at the stop | no | La tabella "Box in" include il cambio di 4 gomme |

Le impostazioni valgono per il bridge o l'hub a cui sei collegato e vengono
salvate in `%APPDATA%\LMUPitwall\strategy.json`.

## Limiti attuali

- Verificato solo con sessioni simulate (`tools/mock-session`), non ancora in
  una gara vera. Da controllare in pista:
  - che la perdita in pit lane misurata sia credibile;
  - che il tempo da fermo calcolato sia vicino alla stima del gioco;
  - che il conteggio dei giri a fine gara sia giusto.
- Le gare a giri usano i giri della tua auto, non quelli del leader.
- Il consumo non tiene conto della mappa motore o del lift and coast scelti:
  si adatta da solo nei giri successivi.
