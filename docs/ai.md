# Commenti AI dell'ingegnere

Il widget **AI engineer** mostra brevi commenti scritti da Claude (Anthropic) sul
momento della gara: tendenze, anomalie, suggerimenti di strategia. Sono per te,
non per il pilota: decidi tu cosa passare via radio.

## Cosa riceve l'AI

Un riepilogo compatto, preparato dal software, non la telemetria grezza:
- piano di strategia: giri alla fine, consumi, soste, finestra box, opzioni
  "box tra N giri";
- gomme (usura, calo di passo, bloccaggi);
- meteo;
- posizione in classe e rivali davanti e dietro;
- eventi di gara dall'ultimo commento.

I dati vanno ai server di Anthropic.

## Nessun numero inventato

All'AI viene chiesto di usare solo i numeri del riepilogo, senza fare conti. Poi
ogni numero del commento viene confrontato con quelli del riepilogo (è ammesso
arrotondarli). Basta un solo numero che non torna e il commento viene
**trattenuto**: il widget scrive *Withheld* e indica quali numeri non tornavano.
Dalle impostazioni puoi scegliere di vederli comunque, segnalati in rosso.

## Attivazione

1. Crea una chiave su [console.anthropic.com](https://console.anthropic.com).
2. Impostala come variabile d'ambiente del PC che fa girare l'hub (o il Pitwall):
   ```
   setx ANTHROPIC_API_KEY "sk-ant-..."
   ```
   Poi chiudi e riapri Pitwall. La chiave resta su quel PC: non viene mai
   mandata alla dashboard né scritta nei log.
3. Nel widget premi **AI off** per passare ad **AI on**.

Se la chiave viene rifiutata, l'AI si spegne da sola e il widget dice perché.

## Quando commenta

- Ogni N giri completati (predefinito: 2), dal primo giro in poi.
- Dopo gli eventi importanti, se attivo *on race events*: uscita dai box,
  full course yellow e ripartenza, penalità, cambio pilota, urti, cambi di
  posizione in classe.
- Quando premi **Ask**, con o senza una domanda ("conviene rientrare adesso?").
  Se è già in corso un'altra richiesta, la domanda parte subito dopo.

Tra due commenti automatici passano almeno *Min interval* secondi (predefinito 60).

**Prove:** in Practice, Test Day e Warmup non ci sono commenti automatici, così
le prove non consumano il budget. **Ask** funziona lo stesso, utile per
provare la chiave prima della gara. Quando inizia una nuova sessione i commenti
precedenti spariscono e non vengono più mandati all'AI come contesto.

## Costi

| Impostazione | Predefinito |
|---|---|
| Model | `claude-opus-5-5` |
| Effort | low |
| Every N laps | 2 |
| Max per hour | 40 |
| Daily budget | 3 $ |

- **Prezzo:** `claude-opus-5-5` costa 4 $ per milione di token in ingresso e
  20 $ in uscita.
- **Per commento:** circa 0,014 $ con riepiloghi di questa dimensione. Il valore
  esatto di ogni commento è nel widget, calcolato dai token reali della risposta.
- **Una 6 ore** con giri da 90 s e un commento ogni 2 giri: circa 120 commenti,
  quindi intorno a 1,7 $.
- **Budget:** raggiunto il budget del giorno, l'AI si ferma fino al giorno dopo.
  La spesa viene salvata in `%APPDATA%\LMUPitwall\ai_usage.json`.
- **Modelli più economici:** si possono scrivere nelle impostazioni. Per esempio
  `claude-haiku-5-5` costa molto meno, con commenti meno accurati.

Se il modello scelto rifiuta per motivi di sicurezza, risponde in automatico un
altro modello (fallback lato server). Un rifiuto finale viene segnalato nel widget.

## Dove gira

L'AI gira dove gira la dashboard che guardi: sull'hub se segui una squadra.
Basta che la chiave sia sul PC dell'hub: i Pitwall dei piloti non ne hanno bisogno.
