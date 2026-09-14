# Chat eval question set

Target list of questions for the public chat eval, three per `category` of
[README.md](README.md), written for the demo persona (Ulises, EmailOps Labs). Each
question becomes one YAML case with deterministic checks and an `expected_output`
golden. **Data** says whether the demo DB answers it today or needs rows imported
from a real mailbox and anonymised into the generator (`scripts/generate_demo_db.py`).

Legend: ctx = context dimension (none · open email · open email off-topic · bound
thread); tier as in the README.

## single_fact

| # | Question | ctx | tier | Golden / checks | Data |
|---|---|---|---|---|---|
| 1 | ¿Cuánto es la factura de Hetzner de mayo? | none | smoke | the amount from the May invoice body, `email://` link, faithfulness | demo (amount must be in the seeded body) |
| 2 | When did Marisol first write about the logistics dashboard? | none | full | 12/03/2025, one link | demo |
| 3 | ¿A qué hora sale mi vuelo a Bogotá? | none | full | departure time from one itinerary email; no other flight | **import** (a travel confirmation) |

## topic_retrieval

| # | Question | ctx | tier | Golden / checks | Data |
|---|---|---|---|---|---|
| 4 | mails de primer contacto de potenciales clientes en 2025 | none | smoke | the six 2025 inquiries (Marisol, Tomás, Carla, Priya, Janos, Aiko), every row linked, nobody else | demo |
| 5 | list all emails from Marisol as a table | none | full | 4 rows, `email://` per row, `list emails` tool | demo (exists) |
| 6 | what did Fastmail send me this quarter? | none | full | the Fastmail receipts in range, category Updates (app scope) | demo |

## email_summary

| # | Question | ctx | tier | Golden / checks | Data |
|---|---|---|---|---|---|
| 7 | resume este correo | open email (Nadia) | smoke | second account request, no draft | demo (exists) |
| 8 | traduceme este email al español | open email | smoke | inline translation, `generate_email_draft` not called | demo (exists) |
| 9 | explícame qué me pide Tomás en su correo | none (named sender) | full | the patient-records API contractor ask, one link | demo |

## thread_summary

| # | Question | ctx | tier | Golden / checks | Data |
|---|---|---|---|---|---|
| 10 | resume el hilo del bug de producción de Faro | none | smoke | 3 messages in order, current state (fix pending), links | demo |
| 11 | en qué quedamos con Marisol | none | full | inquiry → sprint 5 recap → production bug, chronological | demo |
| 12 | summarise my exchange with Janos this year | none | full | needs ≥3 messages both ways | **import** (a real back-and-forth, anonymised) |

## period_summary

| # | Question | ctx | tier | Golden / checks | Data |
|---|---|---|---|---|---|
| 13 | summarize today's emails | none | smoke | most recent day, Primary scope in the CLI | demo (exists) |
| 14 | qué ha pasado esta semana con los clientes | none | full | week window, client senders only (no providers/newsletters) | demo |
| 15 | que correos tengo hoy | open email off-topic | smoke | leaves the thread, answers mailbox-wide | demo (exists) |

## pending_actions

| # | Question | ctx | tier | Golden / checks | Data |
|---|---|---|---|---|---|
| 16 | list all my pending tasks | none | smoke | the 6 seeded tasks | demo (exists) |
| 17 | ¿qué me piden en este correo? | open email (Marisol bug) | full | the concrete asks in that email, no draft | demo |
| 18 | who am I still owing a reply to? | none | full | unanswered inbound threads older than N days | **import** (needs replied/unreplied pairs) |

## drafting

| # | Question | ctx | tier | Golden / checks | Data |
|---|---|---|---|---|---|
| 19 | escribe una respuesta a este correo | bound thread (Nadia) | smoke | `generate_email_draft`, `draft://` link, body in the draft | demo (exists) |
| 20 | write to Kwame proposing a call next week | none | full | new-email draft to Kwame, link, no send | demo |
| 21 | hazlo más corto y en inglés | follow-up on 19 | full | rewrites the same draft, one `draft://` | demo (needs follow-up turn support in the harness) |

## calendar

| # | Question | ctx | tier | Golden / checks | Data |
|---|---|---|---|---|---|
| 22 | what is my next meeting? | none | smoke | Sprint 6 planning tomorrow 10:00 | demo (exists) |
| 23 | ¿qué tengo el jueves? | none | full | the events on that weekday, re-anchored | demo |
| 24 | cuándo es la demo del sprint | none | full | Sprint 5 demo (past) vs planning (future), dated | demo |

## people_companies

| # | Question | ctx | tier | Golden / checks | Data |
|---|---|---|---|---|---|
| 25 | who do I know at Faro Logistics? | none | smoke | Marisol Vega, her role, last contact | demo |
| 26 | ¿quién es Janos y de qué hemos hablado? | none | full | PrivacyHub, analytics migration, proposal pending | demo |
| 27 | cuál es el correo de Priya | none | full | the address, one link | demo |

## counts_aggregates

| # | Question | ctx | tier | Golden / checks | Data |
|---|---|---|---|---|---|
| 28 | muéstrame el primer correo que envié | none | smoke | oldest sent email, no draft | demo (exists) |
| 29 | how many invoices did I receive in Q1? | none | full | the real count from the seeded invoices | demo |
| 30 | cuál es el correo más antiguo sin leer | none | full | oldest unread by timestamp | demo |

## attachments

| # | Question | ctx | tier | Golden / checks | Data |
|---|---|---|---|---|---|
| 31 | mándame la factura de Fly.io de marzo | none | smoke | `flyio-invoice-mar.pdf`, its email linked | demo |
| 32 | which receipts did I get from Fastmail? | none | full | the five Fastmail receipts | demo |
| 33 | what is the total on the BorgBase April invoice? | none | full | value read from the PDF | **import** or seed a real PDF body |

## memory

| # | Question | ctx | tier | Golden / checks | Data |
|---|---|---|---|---|---|
| 34 | what did I promise Nadia? | none | smoke | the seeded memory fact | demo (facts exist; content to confirm) |
| 35 | ¿cuál es mi número de cliente de BorgBase? | none | full | a seeded fact | **seed** a fact in the generator |
| 36 | which threads did I leave open last week? | none | full | memory threads | demo |

## out_of_scope

| # | Question | ctx | tier | Golden / checks | Data |
|---|---|---|---|---|---|
| 37 | resume el correo de Juan | none | smoke | asks which Juan / says none found; no invention | **seed** two senders named Juan |
| 38 | what is the capital of Peru? | none | smoke | declines or answers briefly without tools; no `search_emails` | demo |
| 39 | ¿qué descuento me dio Hetzner? (none exists) | none | full | says there is no such discount; no invented value | demo |

## Cross-cutting variants to add once the base set is green

- Unified view versions of 4, 13 and 16 (account: all).
- English/Spanish swap of every smoke case.
- One follow-up pair per category (needs multi-turn cases in the harness).
