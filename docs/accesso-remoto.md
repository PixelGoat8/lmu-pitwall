# Accesso remoto

Come fa l'ingegnere di pista a seguire da casa la macchina di un amico.

## Come funziona

- Sul PC di chi guida gira **LMU Pitwall** (il "bridge"), accanto al gioco.
- L'ingegnere apre nel browser un **link** che contiene l'indirizzo di quel PC e
  un **token di accesso**. Il link si apre una volta sola: il browser ricorda il token.
- I due PC si raggiungono tramite **Tailscale**, una rete privata tra i vostri
  dispositivi. Non serve aprire porte sul router.

Chi ha il link può **guardare** i dati. Non può spegnere Pitwall, cambiargli
porta, installare voci o fargli leggere cartelle: queste azioni funzionano solo
dal browser del PC di chi guida.

## Preparazione (una volta sola)

### Chi guida

1. Installa [Tailscale](https://tailscale.com/download) e accedi.
2. Condividi il tuo PC con l'ingegnere: dalla pagina *Machines* di Tailscale,
   menu del PC → **Share…** → invia l'invito. In alternativa l'ingegnere ti
   invita nella sua rete. Il piano gratuito ha un limite di utenti: controllalo
   sul sito di Tailscale.
3. Installa e avvia LMU Pitwall. Alla prima apertura Windows può chiedere il
   permesso per la rete: **consenti**, sia per reti private sia pubbliche.
4. In LMU: *Settings → Gameplay → Enable Plugins → ON*, poi riavvia il gioco.

### L'ingegnere

1. Installa Tailscale e accedi.
2. Accetta l'invito ricevuto da chi guida.

## A ogni gara

1. **Chi guida** avvia Pitwall, clicca **🔗** nella barra in alto e copia il
   link *Over Tailscale*. Lo manda all'ingegnere (va bene anche in chat: vedi
   *Sicurezza*).
2. **L'ingegnere** apre il link. Da quel momento basta l'indirizzo senza token,
   perché il browser lo ricorda.
3. Chi guida può chiudere il browser e giocare: Pitwall resta attivo finché il
   gioco è aperto.

Il link cambia solo se cambia il token o l'indirizzo Tailscale del PC, quindi di
solito va mandato una volta sola per ogni pilota.

## Cosa indica il pallino in alto a sinistra

| Scritta | Significato |
|---|---|
| **Live** | Collegato, sessione in corso |
| **Menu** | Collegato, gioco aperto ma nei menu |
| **Bridge** | Collegato a Pitwall, gioco chiuso |
| **…** | Collegamento perso, sto riprovando |
| **Token?** | Pitwall risponde ma rifiuta il token: fatti rimandare il link |
| **Off** | Pitwall non raggiungibile |

La dashboard riprova da sola, ogni 5 secondi al massimo, e si ricollega subito
quando il tablet si risveglia o torna la rete.

## Se non si collega

- **Off**: Pitwall è avviato sul PC di chi guida? Tailscale è acceso su entrambi
  i PC? L'invito è stato accettato?
- **Off solo da remoto, ma in locale funziona**: quasi sempre è il firewall di
  Windows. Nelle app consentite, `lmu-pitwall.exe` deve avere la spunta su
  *Privata* e *Pubblica*.
- **Token?**: il token è cambiato. Fatti mandare di nuovo il link, oppure
  incollalo in *Settings → Connection → Access Token*.
- **Menu anche se l'amico è in pista**: in LMU manca *Enable Plugins*, o il
  gioco non è stato riavviato dopo averlo attivato.

## Sicurezza

- Il token è generato al primo avvio e salvato in
  `%APPDATA%\LMUPitwall\config.json`. Per cambiarlo modifica quel file (o
  cancella la riga `token`) e riavvia Pitwall.
- Su Tailscale il traffico è cifrato. Sulla rete di casa il link viaggia in
  chiaro: usalo solo su reti di cui ti fidi.
- Non mettere davanti a Pitwall un proxy sullo stesso PC (per esempio
  `tailscale serve`): Pitwall tratta come "locale" tutto ciò che arriva dal PC
  stesso, e il proxy farebbe sembrare locali anche le richieste remote.

## Opzioni da riga di comando

| Opzione | Effetto |
|---|---|
| `--token <T>` | Usa questo token invece di quello nel file di configurazione |
| `--keep-alive` | Non chiudere mai Pitwall da solo |
| `--no-browser` | Non aprire il browser all'avvio |
| `--ws-port <N>` | Porta (predefinita 9000) |

Il token si può dare anche con la variabile d'ambiente `LMU_PITWALL_TOKEN`.
Senza `--keep-alive`, Pitwall si chiude dopo 10 minuti senza nessun browser
collegato **e** con il gioco chiuso.
