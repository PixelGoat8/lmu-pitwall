# Hub multi-pilota

Come seguire una gara a stint in cui guidano più amici, ognuno dal suo PC.
Per l'uso quotidiano, con una o più auto, vedi
[guida-ingegnere.md](guida-ingegnere.md); qui ci sono i dettagli.

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

Il pilota finisce nella squadra che stai guardando. L'elenco viene salvato in
`%APPDATA%\LMUPitwall\teams.json` e ricaricato a ogni avvio.

## Più squadre (più auto)

Un hub può seguire più auto nostre nella stessa gara: una **squadra** per auto,
ognuna con i suoi piloti, la sua strategia, i suoi eventi, il suo storico e i
suoi commenti AI, in `%APPDATA%\LMUPitwall\teams\<squadra>`. Tracciati,
perdita in pit lane e budget AI restano in comune.

- La dashboard sceglie la squadra coi pulsanti in alto, o con `?team=<id>`
  nell'indirizzo; senza, mostra la prima.
- Squadre e piloti si gestiscono solo dal PC dell'hub (pannello Team).
- L'auto delle altre squadre è evidenziata col loro colore.
- Un `hub.json` di una versione precedente diventa da solo la prima squadra,
  *Squadra*, con le impostazioni di strategia e AI di prima.

## Accessi

Chi si collega da un altro PC o da un tablet ha un ruolo: **ingegnere** (con
il link 🔗 dell'hub o un link personale) oppure **ospite** (solo guardare, e
solo le sue squadre). Il PC dell'hub è l'amministratore. Le persone con un
link personale sono in `%APPDATA%\LMUPitwall\users.json`. I permessi li
controlla l'hub, non la dashboard: un comando non permesso viene rifiutato con
un messaggio. Come aggiungere una persona: [guida per l'ingegnere](guida-ingegnere.md#un-link-per-persona).

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
- La tabella del pannello vive nella memoria dell'hub: si azzera se chiudi
  l'hub o inizia una nuova sessione. La sessione resta comunque salvata nello
  storico (🗂), uno per squadra: vedi [storico.md](storico.md).

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

Per due auto nostre nella stessa gara, già divise in due squadre, c'è
`tools/mock-session/two-teams.ps1` (vedi [guida-ingegnere.md](guida-ingegnere.md#provare-senza-una-gara)).
