# Storico delle sessioni

Pitwall salva ogni sessione mentre la segui, in
`%APPDATA%\LMUPitwall\history.sqlite` sul PC che la registra: quello del pilota,
l'hub o il tuo PC in modalità engineer. Per rivederla premi **🗂** nella barra in alto.

## Cosa viene salvato

Per ogni sessione, dal primo momento in cui la nostra auto è in pista:

| Dato | Note |
|---|---|
| I nostri giri | tempo, pilota, giro ai box, giro in neutralizzazione, posizione in classe, VE al traguardo; carburante e usura gomme solo dal PC che guidava |
| I giri di tutte le auto | per confrontare i rivali dopo la gara |
| Gli stint | pilota, PC, durata, giri, giro migliore e medio, consumo di carburante e VE |
| Gli eventi | come nel widget *Race events* |
| Il piano di strategia | com'era a ogni giro |
| I commenti AI | compresi quelli trattenuti e il loro costo |
| Le modifiche | chi ha cambiato strategia, target, Pin o AI, e quando (come nel widget *Changes*) |

Una sessione compare nell'elenco dopo il primo giro completato.

Le prove (Practice, Test Day, Warmup) non vengono registrate: Pitwall di solito
parte molto prima della gara e lo storico tiene solo qualifiche e gara. Anche i
commenti AI scritti durante le prove non finiscono nella sessione dopo.

## Il pannello History

- **Elenco:** pista, tipo di sessione, data, giri, piloti, giro migliore. Le
  sessioni seguite dal posto engineer sono segnate *(engineer seat)*: lì
  carburante e usura non c'erano.
- **Stint:** tabella di confronto con giro medio e migliore e consumo per giro,
  più un grafico del giro medio per gli stint di almeno 3 giri.
- **Tempi sul giro:** la nostra auto e fino a 3 rivali a scelta, giro per giro.
  I giri ai box e in neutralizzazione non sono disegnati; i giri molto lenti
  (testacoda, traffico) vengono tagliati in alto per non schiacciare il resto.
- **Eventi**, **modifiche** (chi ha cambiato cosa, con ora e giro) e
  **commenti AI** della sessione.
- **Delete:** cancella una sessione. Funziona solo dal PC che l'ha registrata.

## Dove sono i dati

Su un team con l'hub, lo storico completo è sull'hub, perché segue tutti i
cambi pilota. Ogni Pitwall dei piloti registra comunque le sessioni in cui era
collegato. Il file si può copiare o salvare come qualsiasi altro file.
