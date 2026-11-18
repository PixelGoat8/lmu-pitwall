# Hub multi-pilota

Come seguire una gara a stint in cui guidano più amici, ognuno dal suo PC.

## Come funziona

- Ogni pilota avvia **LMU Pitwall** sul suo PC, come sempre (vedi
  [accesso-remoto.md](accesso-remoto.md)).
- L'ingegnere avvia Pitwall in **modalità hub** sul proprio PC. L'hub si collega
  ai Pitwall di tutti i piloti e mostra sempre quello di chi sta guidando.
- A ogni cambio pilota la dashboard passa da sola al PC del nuovo pilota.
  Lo storico degli stint resta unico per tutta la sessione.

Serve un Pitwall per pilota perché online il gioco mostra carburante, gomme e
freni solo sul PC di chi guida (misurato: vedi [lmu-api.md](lmu-api.md)).

## Avvio

```
lmu-pitwall.exe --hub
```

Si apre la dashboard dell'hub su `http://localhost:9100`. L'hub usa la porta
9100, quindi può stare sullo stesso PC di un Pitwall normale (porta 9000).

## Aggiungere i piloti (una volta sola)

1. Ogni pilota clicca **🔗** nel suo Pitwall e ti manda il link *Over Tailscale*.
2. Nella dashboard dell'hub clicca il riquadro **👥** in alto: compare il
   pannello *Team & Stints*.
3. Incolla il link, scrivi il nome del pilota, premi **Add**.

L'elenco viene salvato in `%APPDATA%\LMUPitwall\hub.json` e ricaricato a ogni avvio.

## Il pannello Team

| Cosa vedi | Significato |
|---|---|
| Pallino verde / giallo / rosso | PC collegato / in collegamento / non raggiungibile o token sbagliato |
| **driving** | Quel PC sta guidando la macchina |
| **watching · car driven by …** | Quel PC è in sessione ma guida un altro |
| **in menus**, **game closed** | Gioco nei menu, o chiuso |
| **ON SCREEN** | È il PC che stai vedendo nella dashboard |

**Following: Auto** segue chi guida. **Pin** blocca la vista su un PC anche se
guida un altro (utile se l'automatismo sbaglia); cliccalo di nuovo per tornare
ad Auto. Durante una sosta, quando nessuno ha la macchina in mano, la vista
resta dov'era.

## Storico degli stint

Uno stint inizia quando la macchina esce dai box e finisce quando ci rientra, o
quando cambia pilota. Per ogni stint: pilota, durata, giri, giro migliore, media
(senza il giro di uscita se lo stint ha almeno tre giri), consumo di carburante
e di VE per giro.

- Il carburante viene letto solo mentre il PC seguito sta guidando: da un altro
  PC il gioco dà un valore fisso.
- Lo storico vive nella memoria dell'hub: si azzera se chiudi l'hub o inizia
  una nuova sessione. Il salvataggio su disco arriva con la fase 6.

## Limiti attuali

- **Come l'hub riconosce chi guida non è ancora verificato in un evento a
  squadre.** L'hub considera "alla guida" il PC su cui la propria auto è sotto
  controllo locale. In garage e da spettatore il valore misurato è diverso,
  come previsto; il caso dell'auto condivisa va provato in una gara vera.
  Il pannello mostra per ogni PC cosa sta facendo, e c'è **Pin** come rimedio.
- *Post Race Results* e *Fuel Calculator* aperti dall'hub usano i dati del PC
  dell'ingegnere, non quelli dei piloti.

## Provare senza il gioco

Il bridge può registrare e riprodurre una sessione:

```
lmu-pitwall.exe --record gara.pwrec                 registra mentre giochi
lmu-pitwall.exe --replay gara.pwrec --ws-port 9001  la riproduce, senza LMU
```

`tools/mock-session/make_session.py` genera sessioni finte (non sono dati reali)
viste da più piloti, per provare l'hub su un solo PC:

```
python tools/mock-session/make_session.py alice.pwrec --me Alice --team "Alice:0-80,Bob:80-150" --duration 150 --lap-time 15
python tools/mock-session/make_session.py bob.pwrec   --me Bob   --team "Alice:0-80,Bob:80-150" --duration 150 --lap-time 15
lmu-pitwall.exe --replay alice.pwrec --ws-port 9141 --token a --no-browser
lmu-pitwall.exe --replay bob.pwrec   --ws-port 9142 --token b --no-browser
lmu-pitwall.exe --hub
```

Poi nell'hub aggiungi `http://<ip-del-pc>:9141/?token=a` e
`http://<ip-del-pc>:9142/?token=b`. Usa l'indirizzo di rete del PC, non
`localhost`.
