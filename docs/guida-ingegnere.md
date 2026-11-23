# LMU Pitwall — guida per l'ingegnere

L'**hub** è il Pitwall dell'ingegnere: si collega ai Pitwall dei piloti e
mostra in una sola pagina le auto della squadra, con strategia, storico e
commenti. **Non serve Le Mans Ultimate** sul PC dell'hub: basta un PC qualsiasi
acceso per tutta la gara.

Per i piloti c'è [guida-pilota.md](guida-pilota.md); i dettagli tecnici
dell'hub sono in [hub-multi-pilota.md](hub-multi-pilota.md).

## Una volta sola

1. **Tailscale** sul PC dell'hub, con lo stesso account a cui i piloti hanno
   condiviso il loro PC (punto 1 della guida pilota).
2. **Pitwall**: installa `LMU-Pitwall-Setup-….exe` e spunta l'icona
   **Pitwall Hub**.
3. Apri **Pitwall Hub**: si apre la pagina `http://localhost:9100`.

## Squadre e piloti

Una **squadra** è un'auto: i suoi piloti, la sua strategia, il suo storico.
Con due auto in gara servono due squadre, ad esempio *Ufficiale* e *Clienti*.

Si gestiscono **solo dal PC dell'hub**, nel pannello **Team** (clic sul
riquadro 👥 in alto):

| Per | Fai così |
|---|---|
| Creare una squadra | In fondo a *Teams* scrivi il nome e premi **Create** |
| Rinominarla | Scrivi nel suo nome e premi Invio |
| Cambiarle colore | Clic sul pallino colorato, poi sul colore |
| Aggiungere un pilota | Scegli in alto la squadra, incolla il link 🔗 del pilota, scrivi il nome, **Add** |
| Spostare un pilota | Menu accanto al suo nome → l'altra squadra |
| Cancellare una squadra | ✕ accanto al nome: solo se è vuota e non è l'ultima |

Squadre e piloti restano salvati da una gara all'altra. Prima di una gara
basta spostare i piloti dove servono.

Se due piloti della stessa squadra sono in pista **su auto diverse**, il
pannello lo segnala: una squadra segue una sola auto, quindi uno dei due va
spostato.

## Durante la gara

- **In alto** c'è un pulsante per squadra, col suo colore: cambia l'auto a
  schermo, il layout resta uguale. Il bordo della barra ha il colore della
  squadra che stai guardando.
- Per tenerle **entrambe sotto controllo** apri due schede del browser, una per
  squadra. Ogni scheda ricorda la sua (`?team=…` nell'indirizzo).
- L'**altra auto** è evidenziata col colore della sua squadra in classifica,
  nei distacchi, nel traffico e sulla mappa. Nei distacchi trovi anche il gap
  verso di lei, anche quando non è l'auto subito davanti o dietro.
- L'hub segue da solo chi guida e cambia PC a ogni cambio pilota. **Pin**
  blocca la vista su un PC se l'automatismo sbaglia.

## Il secondo ingegnere e gli ospiti

Al secondo ingegnere bastano Tailscale e un browser: non installa niente.

1. Su [login.tailscale.com/admin/machines](https://login.tailscale.com/admin/machines),
   accanto al PC dell'hub premi **⋯** → **Share…** e mandagli l'invito.
2. Nella pagina dell'hub premi **🔗** e mandagli il link *Over Tailscale*.
   Aggiungendo `&team=clienti` in fondo, il link apre direttamente quella
   squadra.

Chi apre questo link è un **ingegnere**: guida la gara (strategia, target,
Pin, domande all'AI) per entrambe le squadre, ma **non** gestisce squadre e
piloti e non cambia le impostazioni dell'AI. Quelle restano sul PC dell'hub.

### Un link per persona

Meglio dare a ognuno **il suo link**: si sa chi è collegato e si può togliere
l'accesso a una persona sola. Sul PC dell'hub premi **👥** (*Access*):

1. Scrivi il nome, scegli **Engineer** o **Guest** (per un ospite spunta le
   squadre che può guardare) e premi **Add**.
2. Sotto il suo nome c'è il suo link: **Copy** e mandaglielo.

Dallo stesso pannello cambi ruolo e squadre, dai un link nuovo (**New link**:
quello vecchio smette subito di funzionare) o togli l'accesso (**Remove**: la
persona viene scollegata subito). In fondo, **Connected now** dice chi è
collegato, su quale squadra, da dove e da quanto.

| | Ospite (`guest`) | Ingegnere (`engineer`) | PC dell'hub |
|---|---|---|---|
| Guardare | solo le squadre in `teams` (l'auto delle altre resta evidenziata) | tutte | tutte |
| Strategia e target, Pin, domande all'AI | | ✓ | ✓ |
| Impostazioni e budget dell'AI | | | ✓ |
| Squadre, piloti, persone | | | ✓ |
| Cancellare storico e tracciati | | | ✓ |

Il link dell'hub (🔗) resta valido come accesso da ingegnere. Le persone sono
salvate in `%APPDATA%\LMUPitwall\users.json`; il file si può anche modificare
a mano (`{"name": "Marco", "role": "engineer"}` basta: l'hub aggiunge il
token da solo), e un ruolo scritto male vale come ospite.

## Emergenza: senza i Pitwall dei piloti

Se il Pitwall di un pilota non funziona, ma l'ingegnere ha LMU ed è nella
sessione (posto da ingegnere o spettatore), può leggere l'auto dal proprio
gioco. Per due auto servono due Pitwall sul suo PC, ognuno con la sua
cartella e la sua porta:

```
lmu-pitwall.exe --data-dir "%APPDATA%\LMUPitwall-auto2" --ws-port 9002
```

In ognuno scegli in alto l'auto da seguire (👁 *Follow a car*), poi aggiungilo
all'hub nella squadra giusta col suo link 🔗, come un pilota. Carburante e
gomme non arrivano da qui: il gioco li dà solo a chi guida.

## Provare senza una gara

`tools/mock-session/two-teams.ps1` avvia una gara finta con due auto nostre
(Anna e Bruno, dati inventati) e un hub con due squadre, in una cartella
temporanea: le tue squadre e il tuo storico restano intatti.

```
pwsh tools\mock-session\two-teams.ps1          avvia e apre l'hub (porta 9183)
pwsh tools\mock-session\two-teams.ps1 -Stop    ferma tutto
```

Serve Python. Con il PowerShell di Windows al posto di `pwsh`:
`powershell -ExecutionPolicy Bypass -File tools\mock-session\two-teams.ps1`.
