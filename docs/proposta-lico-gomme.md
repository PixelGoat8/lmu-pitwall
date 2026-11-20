# Proposta: LiCo contro sosta, e piano gomme

Stato: **proposta**. Del primo passo (quanto costa il LiCo in tempo) c'è la
misura in gara, descritta in [strategia.md](strategia.md#lift-and-coast-lico);
il resto non è ancora nel codice. Descrive due aggiunte al
[motore di strategia](strategia.md):

1. **LiCo contro sosta**: durante un'endurance, dire se conviene fare lift and
   coast per risparmiare una sosta, tenendo conto di quanto tempo costa il
   LiCo, di quanto dura la sosta risparmiata e di cosa fanno gli avversari.
2. **Piano gomme**: su tutta la gara, a ogni sosta, cambiare 4 gomme, 2 o
   nessuna (doppio stint), dentro la dotazione di treni e tenendo una riserva.

Le due cose vanno calcolate insieme: il LiCo allunga gli stint, e quindi
cambia quanto devono durare le gomme.

Ipotesi di partenza: nelle nostre gare non ci sono full course yellow né
safety car. La gara è quindi quasi deterministica; gli imprevisti sono le
soste non pianificate (foratura, danni, penalità) e il traffico.

## Cosa c'è già

| Pezzo | Dove | Cosa fa |
|---|---|---|
| Misura del LiCo | `strategy/mod.rs` | Secondi a giro sopra i 72 km/h senza gas né freno |
| Resa del LiCo | `lico_saving_per_s` | Consumo risparmiato per secondo di LiCo |
| Consumo per una sosta in meno | `per_lap_for_stops` | Il consumo massimo con cui bastano *n* soste |
| Tempo da fermo | `stationary_time` | Dai tempi di lavoro del gioco, con i lavori che si sovrappongono |
| Perdita in pit lane | `pit_loss.json` | Misurata a ogni sosta, per pista |
| Usura gomme | `tyre_plan` | Battistrada per gomma, usura per giro, giri al limite, calo di passo |
| Gomme sì o no | `tyre_choice` | Solo per la prossima sosta |
| Giri di tutte le auto | `race/mod.rs` | Ultimi 20 giri di ogni auto, con le soste |
| VE degli avversari | memoria condivisa, `/rest/strategy/usage` | Giro per giro, per le classi con virtual energy (vedi [lmu-api.md](lmu-api.md)) |
| Pedali degli avversari | memoria condivisa | Gas e freno di ogni auto, anche online |

## 1. LiCo contro sosta

### Il conto, solo per la nostra auto

Due piani fino alla bandiera:

| | Piano A, normale | Piano B, LiCo |
|---|---|---|
| Soste | *n* | *n* − 1 |
| Consumo per giro | *c*, misurato | *c'* = `per_lap_for_stops(n − 1)` |
| Tempo sul giro | *t* | *t* + δ (δ = costo del LiCo, **da misurare**) |
| Tempo ai box | *P·n* + *S_A* | *P·(n − 1)* + *S_B* |

- *L*: giri da adesso alla fine fatti in LiCo
- *P*: perdita in pit lane
- *S_A*, *S_B*: tempo da fermo totale delle soste rimaste in ciascun piano

```
ΔT = T_B − T_A = L·δ − [P + (S_A − S_B)]
```

Il piano B conviene se ΔT < 0, cioè se il LiCo costa meno di

```
δ* = (P + S_A − S_B) / L   secondi a giro
```

È il numero da mostrare in pista: *"pareggio 1,4 s/giro, LiCo misurato
0,9 s/giro → conviene, circa 22 s guadagnati"*.

Note:

- **La sosta risparmiata non vale una sosta intera.** Nel piano A l'ultima
  sosta è spesso un rabbocco con solo l'energia per arrivare: dura poco. Il
  piano B, consumando meno, carica anche meno energia in totale. *S_A − S_B*
  si calcola con `stationary_time` su entrambi i piani, non si stima.
- **Prima si decide, meno costa.** Spalmato su più giri, il risparmio per giro
  è più piccolo, e il costo del LiCo cresce più che in proporzione (estremo
  molto peggio di due volte leggero). Il widget mostra anche quanto servirebbe
  iniziando più tardi (*"se inizi tra 10 giri: X s/giro"*) e il giro oltre il
  quale non conviene più.
- **Gare a tempo**: andare più piano riduce un po' i giri da fare, e quindi
  l'energia che serve. Il calcolo dei giri rimasti (`horizon`) va rifatto per
  il piano B col suo passo.
- **Vincoli** che possono escludere il piano B: gomme che non reggono lo stint
  più lungo (vedi la parte 2), stint massimo per pilota, regole della gara.

### Il costo del LiCo (δ)

È la parte difficile. Sul giro intero il tempo balla di qualche decimo
(traffico, un errore in una curva), e il calo delle gomme e l'auto che si
alleggerisce si confondono col LiCo. Una sessione di prova dedicata a
misurarlo non è pratica, quindi il costo va misurato **durante la gara**, dai
giri normali.

**Metodo principale: il confronto per zone.** Il LiCo costa tempo solo in
pochi punti della pista: dove il pilota alza il piede prima della frenata,
fino all'uscita della curva. Invece di guardare il tempo sul giro intero, si
guardano solo quei tratti:

1. Il giro viene diviso in tratti brevi (10 m). Per ogni tratto si misurano
   tempo, carburante consumato e se il pilota stava veleggiando. I tratti si
   mettono in fila in base a **dove si trova l'auto** (la posizione nella
   telemetria, proiettata sulla traccia del primo giro pulito): la distanza
   calcolata dalla velocità sbaglia di qualche metro con la traiettoria (un
   tornante preso un metro più largo è tre metri più lungo), e in frenata
   qualche metro vale centesimi di secondo per tratto. Il carburante dell'auto
   di chi guida si legge di continuo dalla telemetria, non solo sul traguardo.
2. Per ogni tratto c'è un **riferimento**: la mediana dei giri recenti senza
   LiCo in più. Le gomme e il peso cambiano il passo di tutto il giro, zone
   comprese: il riferimento viene portato al passo che il giro ha fuori dalle
   zone.
3. Una **zona di LiCo** è un gruppo di tratti in cui il pilota ha veleggiato
   più del riferimento, dal punto in cui alza il piede all'uscita della curva
   dopo. In ogni zona: secondi persi e carburante risparmiato rispetto al
   riferimento.
4. Le zone con un'auto vicina (40 m davanti, 20 m dietro, dalle posizioni in
   pista di tutte le auto) si scartano: lì il tempo perso può essere traffico,
   non LiCo.
5. Somma delle zone = **costo del LiCo su quel giro** (δ) e carburante
   risparmiato. Giro dopo giro si ottiene la curva *risparmio → secondi persi*.

Il rumore scende molto, perché conta solo qualche centinaio di metri per
curva invece di tutto il giro, e un errore o un doppiaggio altrove non
sporca la misura. Bastano 2–3 giri di LiCo per avere un numero.

C'è anche un vantaggio in più: si vede **in quali curve il LiCo costa meno**
(secondi persi per litro risparmiato). L'ingegnere può dire al pilota dove
alzare il piede e dove no, per ottenere lo stesso risparmio perdendo meno.

**Come si usa per decidere, senza conoscere il costo prima:**

1. Il widget mostra il pareggio δ\*: *"il LiCo conviene se costa meno di
   1,4 s/giro"*.
2. Il pilota inizia il LiCo.
3. Dopo 2–3 giri il confronto per zone dà il costo vero: *"stai perdendo
   0,9 s/giro, conviene"* oppure *"1,8 s/giro, non conviene: smetti"*.

Provare costa poco: il carburante risparmiato in quei giri non è perso, perché
accorcia il rabbocco finale.

**Il profilo si riempie da solo.** Ogni sessione in cui un pilota fa LiCo,
prove libere e gare comprese, lascia le sue zone misurate. Si salvano per
pista, auto e pilota, come la perdita in pit lane. Alla gara dopo c'è già
una stima prima ancora di iniziare.

**Controllo sul giro intero**: la regressione del tempo sul giro sui secondi
di LiCo e sul numero di giro, sui giri puliti, resta come verifica. Se
dice qualcosa di molto diverso dalle zone, il widget lo segnala.

Oggi a occhio: con un pieno facciamo circa +2 giri senza perdere molto, +3
con LiCo estremo. Il confronto per zone serve a trasformarlo in numeri.

Cosa dice chi l'ha misurato (nessuna regola generale, solo casi):

- in LMU a Spa, un giro in LiCo ha risparmiato circa 1,45 L per 0,7 s persi,
  quindi circa 0,5 s per litro, su una sola auto e una sola pista
  ([Coach Dave](https://coachdaveacademy.com/tutorials/mastering-lift-and-coast-how-to-save-fuel-without-sacrificing-speed/));
- una simulazione IndyCar dà, per il 10 % di consumo in meno, da 0,12 s a
  1,7 s a giro secondo come si fa il LiCo
  ([fastest-lap](https://fastest-lap.readthedocs.io/en/latest/blogs/blog_lift_and_coast.html)).

Quindi il costo dipende molto dalla tecnica del pilota: va misurato per
pilota, non solo per pista e auto.

Il gioco ha delle **luci di lift and coast** in auto (corrette nelle versioni
1.1 e 1.2). Il campo `mLiftAndCoastProgress` della memoria condivisa,
oggi non usato, potrebbe essere quello che le accende: se è così, dice dove
il gioco chiede di veleggiare, e la misura del LiCo può partire da lì.

### Gli avversari

Se il LiCo conviene dipende solo da ΔT. Gli avversari dicono **cosa ci
guadagniamo in posizioni**. Per ogni auto della classe:

```
distacco all'arrivo = distacco ora
                    + giri rimasti × (loro passo − nostro passo, piano A o B)
                    + (costo delle loro soste rimaste − costo delle nostre)
```

Il widget mostra la posizione d'arrivo stimata nei due piani: *"A: P4,
B: P3 (passi #7 di circa 12 s)"*, oppure *"ininfluente: davanti +95 s,
dietro −80 s"*.

| Dato sugli avversari | Come |
|---|---|
| Passo | Media dei loro ultimi giri puliti (già in `race/mod.rs`) |
| Soste rimaste e quanto caricheranno | Dalla loro VE e dal loro consumo, per le classi con VE. Senza VE (es. LMP2): dalla lunghezza dei loro stint passati |
| Se fanno LiCo | Stessa misura che usiamo per noi, dai loro pedali |
| Se cambiano gomme | Da quanto restano fermi rispetto al rifornimento che serviva |
| Costo delle loro soste | Perdita in pit lane della pista + tempo da fermo stimato |

Essere superati mentre si fa LiCo non cambia il risultato finale, ma conta
per traffico, aria sporca e difesa della posizione: va mostrato come avviso,
non messo nel conto.

## 2. Piano gomme

### Cosa cambia rispetto a oggi

- **Ogni gomma ha la sua storia.** Oggi un aumento di battistrada su una gomma
  qualsiasi fa ripartire tutto il treno (`current_set`): cambiando 2 gomme,
  il codice crede nuove anche le altre 2.
- **Dotazione**: quanti treni ci sono, quanti ne restano, quanti ne teniamo di
  riserva.
- **Piano su tutta la gara**, non solo sulla prossima sosta.

### Il modello

**Stato**: per ognuna delle 4 gomme montate, battistrada rimasto, giri fatti,
mescola; gomme nuove rimaste; gomme di riserva.

**Le soste** vengono dal piano carburante/VE (A o B). A ogni sosta il
pianificatore sceglie:

| Azione | Quando ha senso |
|---|---|
| Nessun cambio (doppio stint) | L'usura regge altri due stint con margine |
| Solo 2 gomme (un asse) | Un asse consuma molto più dell'altro; risparmia gomme e tempo |
| 4 gomme | Usura al limite, o calo di passo che vale più di circa 12 s nello stint dopo |

**Vincoli**: nessuna gomma sotto il limite di usura (più un margine) prima
del cambio; gomme nuove usate ≤ dotazione − riserva; stint massimo per pilota.

**Limite di usura**: per la nostra esperienza le gomme restano usabili fino al
50–60 % di battistrada. Il limite predefinito del widget oggi è 30 % (*Tyre
limit* nelle impostazioni), troppo basso per noi: va alzato, ed è il limite
che il piano gomme usa per dire se un doppio stint regge.

**Cosa minimizza**: tempo da fermo in più per le gomme + tempo perso al giro
con gomme vecchie (calo di passo × età × giri) + giro di uscita con gomme
fredde, se non ci sono termocoperte.

Le gomme da bagnato non vengono dalla dotazione da asciutto, secondo
[questa guida](https://www.overtake.gg/news/le-mans-ultimate-strategy-guide-how-to-plan-your-pit-stops.2759/):
il piano conta solo le gomme da asciutto.

**Calcolo**: le soste sono poche (5–15) e le azioni 3–4, quindi una
programmazione dinamica sulle soste trova il piano migliore in pochi
millisecondi. Si ricalcola a ogni giro come il resto della strategia.

### Il cambio gomme costa sempre tempo

In LMU il cambio gomme **non** si sovrappone al rifornimento. Dalla patch 2
del marzo 2024 il cambio pilota e il rifornimento vanno insieme, mentre
cambio gomme, riparazioni e freni no
([note della patch](https://lemansultimate.com/patch-2-26th-march-2024-public-notes/)).
Il nostro bridge lo conferma: nei dati del gioco salvati per i test,
`TireTimeConcurrent` vale 0.

Quindi non esistono cambi "gratis": ogni cambio aggiunge il suo tempo alla
sosta. Con i tempi letti dal gioco in una sessione: circa 12 s per 4 gomme e
4,5 s per 2. Il cambio di 2 gomme costa poco più di un terzo di quello da 4,
ed è spesso la scelta giusta per l'asse che consuma di più. I tempi veri
dell'auto vengono sempre da `pitStopTimes`, che il bridge legge già.

La scelta a ogni sosta è sempre la stessa: secondi da fermo adesso contro
secondi persi al giro con gomme vecchie, e gomme rimaste per dopo.

### Cosa mostra

- **Piano treni**: *"S1: nessun cambio · S2: 4 nuove · S3: solo anteriori ·
  S4: nessun cambio, usati 2,5 treni su 3, riserva 1"*.
- **Doppio stint**: *"regge: anteriore sinistra arriva al 58 % (limite 50 %)"*,
  oppure *"no: posteriore sinistra al limite al giro 87, 6 giri prima della
  sosta"*.
- **Riserva**: 1 treno (o 2 gomme) tenuto per gli imprevisti; a un certo
  numero di giri dalla fine la riserva si libera e il piano la usa.
- **Sosta imprevista** (foratura, danni, penalità): il piano si rifà dal
  nuovo stato, con le gomme davvero montate e quelle rimaste. Una foratura
  consuma una gomma della riserva: il piano dice se basta o se il resto della
  gara va rivisto.

### Insieme al LiCo

Il verdetto LiCo usa il piano gomme del piano B, non quello di A. Stint più
lunghi vogliono dire più giri per treno, e una sosta in meno vuol dire
un'occasione di cambio in meno. Esempio: *"B conviene di 18 s, ma serve
cambiare 2 gomme in più a S3 (+4,5 s, già contati)"*.

## Cosa dice la ricerca (novembre 2026)

| Domanda | Risposta | Fonte |
|---|---|---|
| Il cambio gomme si sovrappone al rifornimento? | **No.** Cambio pilota e rifornimento sì; gomme, riparazioni e freni no | [Patch 2, marzo 2024](https://lemansultimate.com/patch-2-26th-march-2024-public-notes/); `TireTimeConcurrent` = 0 nei nostri dati |
| La VE si carica insieme al carburante? | Sì, dalla patch 3 | [RaceDepartment](https://Racedepartment.com/news/comments/82309/), [RaceSimCentral](https://racesimcentral.net/?p=47744) |
| Treni limitati | Sì: numero fisso a inizio gara, secondo durata e regolamento. Sui server ospitati c'è l'opzione *Number of limited tyres available*; nel race weekend la voce *Available tyres* | [Guida LMU](https://guide.lemansultimate.com/hc/en-gb/articles/13210731599119-What-are-limited-tyres), [Configuring Race Weekend](https://guide.lemansultimate.com/hc/en-gb/articles/13210950566927-Configuring-Race-Weekend), [Hosted servers](https://lemansultimate.com/hosted-servers/) |
| Quante gomme si cambiano | Da 0 a 4, e il menu mostra se sono usate o consumate | [Overtake](https://www.overtake.gg/news/le-mans-ultimate-strategy-guide-how-to-plan-your-pit-stops.2759/) |
| Tempo per 2 o 4 gomme | Circa 5 s e 12 s secondo le guide, in linea con i nostri 4,5 s e 12 s; il dato vero è in `pitStopTimes` | [trophi.ai](https://www.trophi.ai/post/how-to-pit-in-le-mans-ultimate-complete-pit-stop-strategy-guide) |
| Gomme da bagnato | Fuori dalla dotazione da asciutto | [Overtake](https://www.overtake.gg/news/le-mans-ultimate-strategy-guide-how-to-plan-your-pit-stops.2759/) |
| Costo del LiCo | Nessuna regola generale; un caso LMU a Spa: circa 0,5 s per litro risparmiato | [Coach Dave](https://coachdaveacademy.com/tutorials/mastering-lift-and-coast-how-to-save-fuel-without-sacrificing-speed/) |
| Luci LiCo del gioco | Esistono, corrette nelle versioni 1.1 e 1.2 | [TheSixthAxis](https://www.thesixthaxis.com/2025/09/24/le-mans-ultimate-update-1-1-adds-elms-championship-and-a-lot-more/), [note 1.2](https://guide.lemansultimate.com/hc/en-gb/articles/14556121957775-V1-2-Update-2) |

Non trovato online:

- se le gomme rimaste sono leggibili dalle API REST o dalla memoria
  condivisa;
- quanto dura il cambio di 1 o 3 gomme (le guide si contraddicono);
- se esiste un'opzione per le termocoperte;
- se FCY e safety car si possono spegnere sul server (per noi non ci sono
  comunque).

## Da verificare nel gioco, prima del codice

Con [`tools/rest-probe`](../tools/rest-probe) e una sessione vera:

1. **Dotazione**: `SESSSET_tires_available_in_garage` corrisponde a
   *Number of limited tyres available*? Le gomme rimaste si leggono da
   `RepairAndRefuel` (menu box) o vanno contate dal bridge a ogni cambio?
2. **Cambio parziale**: quali combinazioni permette il menu (asse, lato,
   gomma singola) e quanto dura ciascuna in `pitStopTimes`. Il menu propone
   gomme usate da rimontare?
3. **Termocoperte** (`SESSSET_tire_warmers`): senza, quanto costa il giro di
   uscita con gomme nuove.
4. **`mLiftAndCoastProgress`**: cosa vale quando si accendono le luci LiCo.
5. **Carburante e VE durante il giro**: `mFuel` dell'auto di chi guida scende
   in modo continuo e con abbastanza precisione da misurare un tratto di 10 m?
   (Il bridge lo usa solo se prende almeno 20 valori diversi in un giro.)
   E `mVirtualEnergy` della nostra auto si aggiorna durante il giro o solo sul
   traguardo? Se solo sul traguardo, il risparmio per zona si misura col
   carburante e si converte in VE col rapporto misurato giro per giro.
6. **VE degli avversari online** nella nostra classe: misurata in LMGT3 a
   Bahrain, da confermare per le altre.

Da sapere dalla squadra: la classe in cui corriamo di solito (VE o
carburante) e quanti treni dà di solito il server.

## Ordine di lavoro proposto

1. Costo del LiCo (δ): confronto per zone durante la gara (**fatto**, in
   `bridge/src/strategy/zones.rs`, da verificare con una gara vera); manca il
   profilo salvato per pista, auto e pilota.
2. Bilancio A contro B in `model.rs`: funzioni pure, con test.
3. Gomme per singola gomma: storia per gomma al posto di `current_set`,
   dotazione e riserva nelle impostazioni.
4. Pianificatore gomme su tutta la gara, usato da entrambi i piani.
5. Proiezione degli avversari: nuovo modulo su VE, passo e soste delle auto
   della classe.
6. Widget: verdetto LiCo, pareggio, posizioni A/B, piano treni.
7. Gara finta: `tools/mock-session` con avversari con VE e strategie diverse,
   cambi di 2 gomme e soste impreviste.

Ogni passo si può provare da solo con una gara finta, come chiede
[CONTRIBUTING](../CONTRIBUTING.md).
