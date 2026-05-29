# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""
Generate a self-contained EmailOps demo database for screen recordings.

The demo DB lives at:
    ~/Library/Application Support/com.emailops.app-demo/emailops.db

Launch the app against it with `make demo` (sets EMAILOPS_DATA_DIR).

What it does
------------
1. Copies the *schema only* from the current production DB into a fresh demo DB
   so the demo always matches whatever the app currently expects.
2. Populates it with synthetic-but-plausible founder-flavored data:
   - 2 accounts (one Gmail, one Outlook)
   - ~180 emails across realistic SaaS/customer/investor/newsletter senders
   - Read/unread mix, multiple threads, categories, mailboxes
   - email_bodies + emails_fts entries
   - A handful of tags, attachments meta, sync_state, ai_config,
     user_preferences (onboarding_completed=true), pending_tasks, memory_facts

Run:
    uv run scripts/generate_demo_db.py
    # or override targets:
    uv run scripts/generate_demo_db.py \
        --prod-db "$HOME/Library/Application Support/com.emailops.app/emailops.db" \
        --demo-db "$HOME/Library/Application Support/com.emailops.app-demo/emailops.db"
"""

from __future__ import annotations

import argparse
import json
import os
import random
import re
import shutil
import sqlite3
import subprocess
import sys
import time
import uuid
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable

HOME = Path.home()
DEFAULT_PROD_DB = HOME / "Library/Application Support/com.emailops.app/emailops.db"
DEFAULT_DEMO_DIR = HOME / "Library/Application Support/com.emailops.app-demo"
DEFAULT_DEMO_DB = DEFAULT_DEMO_DIR / "emailops.db"
# Spanish demo lives in its own data dir so it never clobbers the English one
# and can be launched independently from `make demo-es`.
DEFAULT_DEMO_DIR_ES = HOME / "Library/Application Support/com.emailops.app-demo-es"
DEFAULT_DEMO_DB_ES = DEFAULT_DEMO_DIR_ES / "emailops.db"

RNG = random.Random(42)  # deterministic demo data


# ──────────────────────────────────────────────────────────────────────────────
# Personas / accounts
# ──────────────────────────────────────────────────────────────────────────────

@dataclass
class Account:
    id: str
    provider: str
    email: str
    name: str


ACCOUNT_WORK = Account(
    id="demo-acct-work",
    provider="gmail",
    email="alex@northwindlabs.io",
    name="Alex Reyes (Northwind Labs)",
)
ACCOUNT_PERSONAL = Account(
    id="demo-acct-personal",
    provider="outlook",
    email="demo-alex.reyes@outlook.com",
    name="Alex Reyes",
)

# Spanish-locale variant. Same personas, Spanish-flavored email/name. Account ids
# stay distinct so a user running both demos doesn't conflict if they ever land
# in the same DB.
ACCOUNT_WORK_ES = Account(
    id="demo-acct-work-es",
    provider="gmail",
    email="jose@vientonorte.io",
    name="José Pérez (Viento Norte)",
)
ACCOUNT_PERSONAL_ES = Account(
    id="demo-acct-personal-es",
    provider="gmail",
    email="demo-jose.perez@example.com",
    name="José Pérez",
)


# ──────────────────────────────────────────────────────────────────────────────
# Email templates (sender, subject, body, category, mailbox, tags)
# Bodies are short — realistic for a SaaS founder's inbox.
# ──────────────────────────────────────────────────────────────────────────────

# (sender_name, sender_email, subject, body, category, weight)
WORK_TEMPLATES: list[tuple[str, str, str, str, str, int]] = [
    # SaaS / product notifications
    ("Stripe", "notifications@stripe.com",
     "Your weekly Stripe summary",
     "Hi Alex,\n\nLast week you processed $12,481.20 across 84 successful payments. "
     "Net volume after refunds was $11,902.40.\n\nView the full report in your dashboard.\n\nThe Stripe team",
     "updates", 3),
    ("Linear", "notifications@linear.app",
     "[NWL-412] Auth: refresh tokens rotating too aggressively",
     "Priya commented:\n\n> I traced this back to the JWT exp we shipped on Tuesday. "
     "I'll open a PR tonight with a fix and a regression test.\n\nView in Linear",
     "updates", 4),
    ("GitHub", "notifications@github.com",
     "[northwind-labs/api] PR #284 ready for review",
     "@alex-reyes opened a pull request:\n\nfix(billing): handle Stripe webhook idempotency keys correctly\n\n"
     "+128 −34 across 6 files. Reviewers: priya-r, marco-d.",
     "updates", 4),
    ("Vercel", "no-reply@vercel.com",
     "Deployment ready: northwind-marketing@main",
     "Your production deployment for northwind-marketing finished successfully.\n\n"
     "Duration: 47s\nCommit: chore: bump pricing copy for Q2\nURL: https://northwindlabs.io",
     "updates", 3),
    ("Sentry", "noreply@sentry.io",
     "[Northwind] New issue: TypeError in checkout.tsx",
     "A new issue was seen 4 times in the last hour.\n\n"
     "TypeError: Cannot read properties of undefined (reading 'plan')\n"
     "at CheckoutForm (src/checkout.tsx:142)",
     "updates", 2),
    ("Notion", "team@mail.notion.so",
     "Marco mentioned you in 'Q2 roadmap draft'",
     "> @alex — can we lock the pricing experiment scope before Thursday's planning? "
     "Happy to drive if you give the thumbs up.",
     "primary", 3),
    ("Figma", "no-reply@figma.com",
     "Priya shared 'Onboarding v3' with you",
     "Priya Raman shared a Figma file with you and left a comment:\n\n"
     "> First pass on the empty-state flow. Curious what you think before I hand off to eng.",
     "primary", 2),
    ("Slack", "noreply@slack.com",
     "You have 3 new messages in #founders",
     "Most recent:\n\n"
     "marco: anyone else seeing the staging deploy hang on the migrations step?\n"
     "priya: yep, looking now — it's the new index on emails(account_id, mailbox)\n"
     "marco: 🙏",
     "social", 2),
    # Customers
    ("Maya Klein", "maya@brightpath.studio",
     "Re: Onboarding for the design team",
     "Hey Alex,\n\nWe finished migrating the last two designers over this morning. "
     "One small thing — when we invite someone via SSO they land on a 404 the first time. "
     "Refreshing fixes it. Not blocking, just FYI.\n\nThanks for the responsive support, "
     "you've made this rollout shockingly smooth.\n\nMaya\nHead of Design, Brightpath Studio",
     "primary", 5),
    ("Daniel Osei", "daniel.osei@helixrobotics.com",
     "Quick question on the enterprise plan",
     "Hi Alex,\n\nWe're evaluating Northwind for a 60-seat rollout next quarter. Two things:\n\n"
     "1. Does the audit log include API key usage events?\n"
     "2. Can we self-host the data residency option in Frankfurt?\n\n"
     "Happy to jump on a call if easier.\n\nDaniel\nVP Eng, Helix Robotics",
     "primary", 5),
    ("Sophie Tran", "sophie@kitewave.io",
     "Bug report: CSV export truncating at 10k rows",
     "Hey team,\n\nFiling this here so it doesn't get lost in chat. When I export the "
     "'All contacts' view to CSV, the file always ends at exactly 10,000 rows even though "
     "the UI says 12,847 records. Repro'd in Chrome and Safari.\n\nUrgent-ish for us — we "
     "send the file to our partner weekly.\n\nSophie",
     "primary", 4),
    ("Lena Park", "lena.park@orbitfreight.co",
     "Re: Renewal terms",
     "Alex,\n\nLegal pushed back on the 3-year commit. They'll sign for 2 years at the same "
     "rate, or 3 years with a 12% discount. Can we make the 12% work?\n\n"
     "Need to close this by Friday to hit our procurement cycle.\n\nLena",
     "primary", 4),

    # Investors / partners
    ("Jordan Wei", "jordan@northstar.vc",
     "Catch-up next week?",
     "Alex, want to grab 30 min next week for a quick check-in? Curious to hear how the "
     "self-serve PLG motion landed and what the early conversion numbers look like.\n\n"
     "Tues or Thurs afternoon works on my end.\n\nJordan",
     "primary", 3),
    ("Investor Update", "lps@northstar.vc",
     "Northstar VC: Q1 portfolio update is live",
     "Our Q1 portfolio letter is available in the LP portal. Highlights:\n\n"
     "• 3 new investments (Lattice AI, Polymer, Trellis)\n"
     "• 2 follow-ons including Northwind Labs Seed extension\n"
     "• Fund III deployment now at 47%",
     "updates", 1),

    # Recruiting / hiring
    ("Greenhouse", "no-reply@greenhouse.io",
     "New candidate for Senior Backend Engineer",
     "A candidate has applied for the Senior Backend Engineer role:\n\n"
     "Name: Rafael Mendes\nLocation: Lisbon, Portugal\nExperience: 8 years (Rust, Go, distributed systems)\n"
     "Source: Hacker News 'Who is Hiring'",
     "updates", 3),
    ("Hannah Bloom", "hannah@northbridgerecruiting.com",
     "Senior eng candidate — 6 yrs Rust, ex-Cloudflare",
     "Hi Alex,\n\nI have a candidate I think is a strong fit for the backend role you posted "
     "last month. Currently at Cloudflare on the workers runtime team. Open to remote-EU.\n\n"
     "CV attached. Let me know if you'd like an intro.\n\nHannah",
     "primary", 2),

    # Newsletters
    ("Lenny's Newsletter", "lenny@substack.com",
     "How Notion went from 0 to 100M users",
     "An interview with Notion's former Head of Growth on the playbook they used to grow "
     "from product-led obscurity to one of the most-loved tools on the internet. (28 min read)",
     "promotions", 2),
    ("Hacker Newsletter", "kale@hackernewsletter.com",
     "Hacker Newsletter #644",
     "This week's top stories:\n\n"
     "• Show HN: I built a local-first email client (Tauri + Rust + Ollama)\n"
     "• SQLite's 'fast' insert mode is faster than you think\n"
     "• A pragmatic guide to llm-as-judge evaluation",
     "promotions", 2),
    ("Y Combinator", "events@ycombinator.com",
     "YC Startup School: AI Agents in production (Apr 22)",
     "Join us for a 4-hour deep dive with founders from Cursor, Cognition, and Replit on what "
     "actually works (and doesn't) when shipping AI agents at scale.\n\nFree, virtual, recording available.",
     "promotions", 1),

    # Cold outreach / sales (spam-ish)
    ("Tyler Brooks", "tyler.brooks@growthengine.io",
     "Quick question, Alex",
     "Hi Alex,\n\nNoticed Northwind Labs is growing fast — congrats on the recent fundraise! "
     "We help Series A SaaS companies improve inbound conversion by 30-40% using AI-powered "
     "lead scoring.\n\nWould you be open to a 15-min call this week?\n\nBest, Tyler",
     "promotions", 3),
    ("Megan Cole", "megan@b2bleads.co",
     "Re: Re: following up",
     "Alex — bumping this to the top of your inbox in case it got buried. Still happy to send "
     "over the case study if you're curious.\n\nMegan",
     "promotions", 2),

    # Calendar / scheduling
    ("Calendly", "notifications@calendly.com",
     "New event: 30min with Daniel Osei (Helix Robotics)",
     "A new event has been scheduled:\n\nEvent: Northwind ↔ Helix discovery call\n"
     "Invitee: Daniel Osei (daniel.osei@helixrobotics.com)\n"
     "Date & time: Thursday at 3:00 PM PT (Zoom)",
     "updates", 2),
    ("Google Calendar", "calendar-notification@google.com",
     "Reminder: Founders sync (in 30 min)",
     "You have an upcoming event in 30 minutes:\n\nFounders sync\nWhen: Today, 10:00 - 10:30 AM\n"
     "Where: Zoom",
     "updates", 2),
]

PERSONAL_TEMPLATES: list[tuple[str, str, str, str, str, int]] = [
    ("Mom", "elena@example.com",
     "Tía Carmen's birthday next weekend",
     "Hi mijo,\n\nDon't forget — Tía Carmen turns 70 on Saturday. We're doing dinner at her place at 7. "
     "Can you bring the wine? Something nice.\n\nLove,\nMom",
     "primary", 4),
    ("Daniel", "demo-daniel@example.com",
     "That book you mentioned",
     "Yo — finally finished 'Working in Public'. You were right, the chapter on maintainer burnout "
     "is incredible. Lending it to anyone? Got two friends asking.",
     "primary", 3),
    ("Airbnb", "noreply@airbnb.com",
     "Your reservation in Lisbon is confirmed",
     "Your trip to Lisbon, Portugal is confirmed.\n\nCheck-in: Friday, May 8\nCheck-out: Monday, May 11\n"
     "Host: Inês\nTotal: €612.40",
     "updates", 2),
    ("Spotify", "no-reply@spotify.com",
     "Your year in music so far",
     "Your top artist this year is Khruangbin. You've listened to 142 hours of music — that's more than "
     "92% of listeners in your country.",
     "promotions", 1),
    ("Strava", "no-reply@strava.com",
     "Your weekly running summary",
     "This week you ran 32.4 km across 4 runs — your highest week this year. Your longest run was "
     "11.2 km on Saturday morning.",
     "updates", 2),
    ("Amazon", "auto-confirm@amazon.com",
     "Your order has shipped",
     "Your order containing 'The Pragmatic Programmer (20th Anniversary Edition)' has shipped and is "
     "expected to arrive on Friday.",
     "updates", 2),
    ("REI", "newsletter@notifications.rei.com",
     "Member dividend ready — $84.20",
     "Your 2025 member dividend is ready. Apply it to any in-store or online purchase before December.",
     "promotions", 1),
    ("Sara Chen", "demo-sara-chen@example.com",
     "Coffee Saturday?",
     "Hey! I'm finally back in town for the weekend. Free Saturday morning? There's that new spot in "
     "the Mission with the wood-fired pastries. My treat.\n\nSara",
     "primary", 3),
    ("Doctor Park", "noreply@onepatient.health",
     "Annual checkup reminder",
     "Hi Alex,\n\nYou're due for your annual physical with Dr. Park. Please book a time at your "
     "convenience using the patient portal link below.",
     "updates", 1),
]


# ──────────────────────────────────────────────────────────────────────────────
# Spanish templates — Viento Norte is the localized startup. Senders, subjects,
# and bodies are Spanish so the entire inbox reads native in a recording.
# Vendor brand names stay in English (Stripe, GitHub, etc. — they're brand
# names) but their notification copy is Spanish.
# ──────────────────────────────────────────────────────────────────────────────

WORK_TEMPLATES_ES: list[tuple[str, str, str, str, str, int]] = [
    # SaaS / product notifications
    ("Stripe", "notifications@stripe.com",
     "Tu resumen semanal de Stripe",
     "Hola José,\n\nLa semana pasada procesaste 12.481,20 € en 84 pagos exitosos. "
     "El volumen neto tras reembolsos fue de 11.902,40 €.\n\nConsulta el informe completo en tu panel.\n\nEl equipo de Stripe",
     "updates", 3),
    ("Linear", "notifications@linear.app",
     "[VTN-412] Auth: los refresh tokens rotan demasiado agresivamente",
     "Priya comentó:\n\n> Lo rastreé hasta el JWT exp que desplegamos el martes. "
     "Abro un PR esta noche con el fix y un test de regresión.\n\nVer en Linear",
     "updates", 4),
    ("GitHub", "notifications@github.com",
     "[viento-norte/api] PR #284 listo para revisar",
     "@jose-perez abrió un pull request:\n\nfix(billing): manejar correctamente las idempotency keys del webhook de Stripe\n\n"
     "+128 −34 en 6 archivos. Revisores: priya-r, marco-d.",
     "updates", 4),
    ("Vercel", "no-reply@vercel.com",
     "Despliegue listo: viento-norte-marketing@main",
     "Tu despliegue en producción de viento-norte-marketing terminó correctamente.\n\n"
     "Duración: 47s\nCommit: chore: actualizar copy de precios Q2\nURL: https://vientonorte.io",
     "updates", 3),
    ("Sentry", "noreply@sentry.io",
     "[Viento Norte] Nueva incidencia: TypeError en checkout.tsx",
     "Se ha registrado una nueva incidencia 4 veces en la última hora.\n\n"
     "TypeError: Cannot read properties of undefined (reading 'plan')\n"
     "en CheckoutForm (src/checkout.tsx:142)",
     "updates", 2),
    ("Notion", "team@mail.notion.so",
     "Marco te mencionó en 'Roadmap Q2 (borrador)'",
     "> @jose — ¿podemos cerrar el alcance del experimento de precios antes "
     "del planning del jueves? Me ocupo yo si me das el visto bueno.",
     "primary", 3),
    ("Figma", "no-reply@figma.com",
     "Priya compartió 'Onboarding v3' contigo",
     "Priya Raman compartió un archivo de Figma contigo y dejó un comentario:\n\n"
     "> Primer pase del flujo de empty-state. Curiosa por tu opinión antes de pasárselo a ingeniería.",
     "primary", 2),
    ("Slack", "noreply@slack.com",
     "Tienes 3 mensajes nuevos en #fundadores",
     "Más reciente:\n\n"
     "marco: ¿alguien más ve que el deploy de staging se cuelga en el paso de migraciones?\n"
     "priya: sí, mirándolo — es el índice nuevo en emails(account_id, mailbox)\n"
     "marco: 🙏",
     "social", 2),
    # Customers
    ("Maya Klein", "maya@brightpath.studio",
     "Re: Onboarding para el equipo de diseño",
     "Hola José,\n\nEsta mañana terminamos de migrar a los dos últimos diseñadores. "
     "Una cosa pequeña — cuando invitamos a alguien por SSO, la primera vez aterriza en un 404. "
     "Recargando se arregla. No bloquea, solo aviso.\n\nGracias por el soporte tan ágil, "
     "has hecho que este rollout sea increíblemente fluido.\n\nMaya\nHead of Design, Brightpath Studio",
     "primary", 5),
    ("Daniel Osei", "daniel.osei@helixrobotics.com",
     "Pregunta rápida sobre el plan enterprise",
     "Hola José,\n\nEstamos evaluando Viento Norte para un despliegue de 60 puestos el próximo trimestre. Dos cosas:\n\n"
     "1. ¿El audit log incluye eventos de uso de API keys?\n"
     "2. ¿Podemos autohospedar la opción de residencia de datos en Frankfurt?\n\n"
     "Encantado de saltar a una llamada si es más fácil.\n\nDaniel\nVP Eng, Helix Robotics",
     "primary", 5),
    ("Sophie Tran", "sophie@kitewave.io",
     "Reporte de bug: el export CSV se trunca en 10k filas",
     "Hola equipo,\n\nLo reporto por aquí para que no se pierda en el chat. Cuando exporto la "
     "vista 'Todos los contactos' a CSV, el archivo siempre termina exactamente en 10.000 filas aunque "
     "la UI dice 12.847 registros. Reproducido en Chrome y Safari.\n\nUn poco urgente — enviamos "
     "el archivo a nuestro partner semanalmente.\n\nSophie",
     "primary", 4),
    ("Lena Park", "lena.park@orbitfreight.co",
     "Re: Términos de renovación",
     "José,\n\nLegal rechazó el compromiso de 3 años. Firmarían 2 años a la misma tarifa, "
     "o 3 años con un 12% de descuento. ¿Podemos hacer que funcione el 12%?\n\n"
     "Necesito cerrar esto antes del viernes para encajar en su ciclo de compras.\n\nLena",
     "primary", 4),

    # Investors / partners
    ("Jordan Wei", "jordan@northstar.vc",
     "¿Nos vemos la próxima semana?",
     "José, ¿quieres tomarte 30 min la próxima semana para ponernos al día? Tengo curiosidad "
     "por saber cómo aterrizó la motion de PLG self-serve y qué números de conversión iniciales estás viendo.\n\n"
     "Martes o jueves por la tarde me viene bien.\n\nJordan",
     "primary", 3),
    ("Investor Update", "lps@northstar.vc",
     "Northstar VC: el update trimestral Q1 ya está publicado",
     "Nuestra carta trimestral Q1 está disponible en el portal de LPs. Lo destacado:\n\n"
     "• 3 nuevas inversiones (Lattice AI, Polymer, Trellis)\n"
     "• 2 follow-ons, incluyendo la extensión de la ronda Seed de Viento Norte\n"
     "• El despliegue del Fondo III está ya al 47%",
     "updates", 1),

    # Hiring
    ("Greenhouse", "no-reply@greenhouse.io",
     "Nuevo candidato para Senior Backend Engineer",
     "Un candidato se ha postulado para la posición de Senior Backend Engineer:\n\n"
     "Nombre: Rafael Mendes\nUbicación: Lisboa, Portugal\nExperiencia: 8 años (Rust, Go, sistemas distribuidos)\n"
     "Fuente: 'Who is Hiring' de Hacker News",
     "updates", 3),
    ("Hannah Bloom", "hannah@northbridgerecruiting.com",
     "Candidato senior — 6 años de Rust, ex-Cloudflare",
     "Hola José,\n\nTengo un candidato que creo encaja muy bien para la posición de backend "
     "que abristeis el mes pasado. Actualmente en Cloudflare en el equipo de runtime de workers. "
     "Abierto a remote-EU.\n\nCV adjunto. Avísame si quieres una introducción.\n\nHannah",
     "primary", 2),

    # Newsletters
    ("Lenny's Newsletter", "lenny@substack.com",
     "Cómo Notion pasó de 0 a 100M de usuarios",
     "Una entrevista con la ex-responsable de Crecimiento de Notion sobre el playbook que usaron para crecer "
     "de la oscuridad product-led a una de las herramientas más queridas de internet. (28 min de lectura)",
     "promotions", 2),
    ("Hacker Newsletter", "kale@hackernewsletter.com",
     "Hacker Newsletter #644",
     "Lo más destacado de esta semana:\n\n"
     "• Show HN: he construido un cliente de email local-first (Tauri + Rust + Ollama)\n"
     "• El modo 'fast' de inserción de SQLite es más rápido de lo que crees\n"
     "• Guía pragmática para la evaluación llm-as-judge",
     "promotions", 2),

    # Cold outreach
    ("Tyler Brooks", "tyler.brooks@growthengine.io",
     "Pregunta rápida, José",
     "Hola José,\n\nVi que Viento Norte está creciendo rápido — ¡enhorabuena por la ronda reciente! "
     "Ayudamos a SaaS en Serie A a mejorar la conversión inbound un 30-40% usando scoring de leads "
     "con IA.\n\n¿Estarías abierta a una llamada de 15 min esta semana?\n\nUn saludo, Tyler",
     "promotions", 3),
    ("Megan Cole", "megan@b2bleads.co",
     "Re: Re: dándole seguimiento",
     "José — subiendo este hilo al principio por si se ha quedado enterrado. Sigo encantada "
     "de enviarte el caso de estudio si te interesa.\n\nMegan",
     "promotions", 2),

    # Calendar
    ("Calendly", "notifications@calendly.com",
     "Nuevo evento: 30min con Daniel Osei (Helix Robotics)",
     "Se ha agendado un nuevo evento:\n\nEvento: Llamada de descubrimiento Viento Norte ↔ Helix\n"
     "Invitado: Daniel Osei (daniel.osei@helixrobotics.com)\n"
     "Fecha y hora: Jueves a las 16:00 CET (Zoom)",
     "updates", 2),
    ("Google Calendar", "calendar-notification@google.com",
     "Recordatorio: Sync de fundadores (en 30 min)",
     "Tienes un evento próximo en 30 minutos:\n\nSync de fundadores\nCuándo: Hoy, 10:00 - 10:30\n"
     "Dónde: Zoom",
     "updates", 2),
]

PERSONAL_TEMPLATES_ES: list[tuple[str, str, str, str, str, int]] = [
    ("Mamá", "elena@example.com",
     "El cumpleaños de la tía Carmen el finde que viene",
     "Hola mijo,\n\nNo se te olvide — la tía Carmen cumple 70 el sábado. Cenamos en su casa a las 21. "
     "¿Puedes traer tú el vino? Algo bueno.\n\nUn beso,\nMamá",
     "primary", 2),
    ("Daniel", "demo-daniel@example.com",
     "El libro que me recomendaste",
     "Eh — por fin terminé 'Working in Public'. Tenías razón, el capítulo sobre el burnout de los "
     "maintainers es increíble. ¿Se lo prestas a alguien? Dos amigos me lo están pidiendo.",
     "primary", 3),
    ("Airbnb", "noreply@airbnb.com",
     "Tu reserva en Lisboa está confirmada",
     "Tu viaje a Lisboa, Portugal está confirmado.\n\nCheck-in: Viernes 8 de mayo\nCheck-out: Lunes 11 de mayo\n"
     "Anfitriona: Inês\nTotal: 612,40 €",
     "updates", 2),
    ("Spotify", "no-reply@spotify.com",
     "Tu año en música hasta ahora",
     "Tu artista número uno este año es Khruangbin. Has escuchado 142 horas de música — "
     "más que el 92% de oyentes de tu país.",
     "promotions", 1),
    ("Strava", "no-reply@strava.com",
     "Tu resumen semanal de carrera",
     "Esta semana corriste 32,4 km en 4 entrenamientos — tu mejor semana del año. "
     "Tu carrera más larga fue de 11,2 km el sábado por la mañana.",
     "updates", 2),
    ("Amazon", "auto-confirm@amazon.com",
     "Tu pedido ha sido enviado",
     "Tu pedido que contiene 'The Pragmatic Programmer (20th Anniversary Edition)' ha sido enviado "
     "y se espera que llegue el viernes.",
     "updates", 2),
    ("Sara Chen", "demo-sara-chen@example.com",
     "¿Café el sábado?",
     "¡Hola! Por fin estoy de vuelta en la ciudad este finde. ¿Libre el sábado por la mañana? "
     "Hay un sitio nuevo en Malasaña con bollería de horno de leña. Invito yo.\n\nSara",
     "primary", 3),
    ("Doctora Park", "noreply@onepatient.health",
     "Recordatorio: revisión anual",
     "Hola José,\n\nTe toca tu revisión anual con la doctora Park. Por favor reserva hora "
     "cuando te venga bien usando el enlace al portal del paciente.",
     "updates", 1),
]


# ──────────────────────────────────────────────────────────────────────────────
# Tag types stored in email_tags. Mirrors classify_intents / classify_topics
# in user_preferences so the UI's filter chips have something to show.
# ──────────────────────────────────────────────────────────────────────────────

INTENT_BY_KEYWORD: list[tuple[str, str]] = [
    # English
    ("invoice", "billing"),
    ("bill", "billing"),
    ("renewal", "billing"),
    ("pricing", "billing"),
    ("bug", "complaint"),
    ("issue", "complaint"),
    ("error", "complaint"),
    ("question", "question"),
    ("?", "question"),
    ("congrats", "feedback"),
    ("thanks", "feedback"),
    ("intro", "introduction"),
    ("introduce", "introduction"),
    ("call", "scheduling"),
    ("meeting", "scheduling"),
    ("scheduled", "scheduling"),
    ("shipped", "delivery"),
    ("deployment", "delivery"),
    ("delivered", "delivery"),
    ("approve", "approval"),
    ("sign", "approval"),
    # Spanish — internal tag values stay canonical (billing/complaint/...),
    # only the keywords change. Matching is substring + lowercase so accents in
    # the source text don't break detection here.
    ("factura", "billing"),
    ("facturación", "billing"),
    ("renovación", "billing"),
    ("precios", "billing"),
    ("bug", "complaint"),
    ("incidencia", "complaint"),
    ("reporte", "complaint"),
    ("pregunta", "question"),
    ("¿", "question"),
    ("enhorabuena", "feedback"),
    ("gracias", "feedback"),
    ("introducción", "introduction"),
    ("presentar", "introduction"),
    ("llamada", "scheduling"),
    ("reunión", "scheduling"),
    ("agendado", "scheduling"),
    ("enviado", "delivery"),
    ("despliegue", "delivery"),
    ("entregado", "delivery"),
    ("aprobar", "approval"),
    ("firmar", "approval"),
]

TOPIC_BY_SENDER_SUBSTR: list[tuple[str, str]] = [
    ("stripe", "billing"),
    ("aws", "billing"),
    ("vercel", "operations"),
    ("github", "project"),
    ("linear", "project"),
    ("notion", "project"),
    ("figma", "project"),
    ("sentry", "operations"),
    ("slack", "operations"),
    ("calendly", "operations"),
    ("calendar", "operations"),
    ("greenhouse", "hiring"),
    ("recruit", "hiring"),
    ("vc", "finance"),
    ("northstar", "finance"),
    ("substack", "marketing"),
    ("newsletter", "marketing"),
    ("ycombinator", "education"),
    ("airbnb", "travel"),
    ("spotify", "personal"),
    ("strava", "personal"),
    ("amazon", "personal"),
    ("rei", "personal"),
    ("onepatient", "personal"),
]


def infer_intent(subject: str, body: str) -> str:
    haystack = (subject + " " + body).lower()
    for kw, intent in INTENT_BY_KEYWORD:
        if kw in haystack:
            return intent
    return "notification"


def infer_topic(sender_email: str) -> str:
    s = sender_email.lower()
    for substr, topic in TOPIC_BY_SENDER_SUBSTR:
        if substr in s:
            return topic
    return "operations"


# ──────────────────────────────────────────────────────────────────────────────
# Locale bundle — everything that differs between en/es lives here. Functions
# below take a `Locale` so the rest of the schema/inserts can be locale-agnostic.
# ──────────────────────────────────────────────────────────────────────────────

@dataclass
class Locale:
    code: str                      # "en" | "es"
    work: Account
    personal: Account
    work_templates: list[tuple[str, str, str, str, str, int]]
    personal_templates: list[tuple[str, str, str, str, str, int]]
    # Vendor invoice / receipt templates that double as attachment-rule fodder.
    # Same tuple shape as work_templates but kept separate so each entry can be
    # paired with a deterministic `(filename, size)` for the attachment row.
    # `(sender_name, sender_email, subject, body, category, weight, filename, size)`
    invoice_templates: list[tuple[str, str, str, str, str, int, str, int]]
    # Strings used when synthesizing replies / sent-mailbox emails.
    user_first_name: str           # e.g. "Alex" → signs replies
    sent_subject_prefix: str       # e.g. "Re: "
    reply_first: str               # body of the first synthesized user reply
    reply_followup: str            # body of the synthesized vendor follow-up
    sent_short: str                # body for the "user just sent" mailbox swap
    # Tasks / memory facts / attachments rendered for this locale.
    tasks: list[tuple[str, str | None, str, str | None, int | None]]
    memory_facts: list[tuple[str, str, str, str]]
    # `(subject_like, filename, file_size)` — attachments derived from email subjects.
    attachments: list[tuple[str, str, int]]
    # Attachment rules so the auto-extract-invoice flow has something to show.
    # `(name, sender_email_pattern, subject_pattern, filename_pattern, tags)`
    attachment_rules: list[tuple[str, str | None, str | None, str | None, list[str]]]
    # The output language pref the app uses for AI responses in the chat.
    ai_output_language: str        # "English" | "Spanish"


def now_plus_days(days: int) -> int:
    """Unix epoch in seconds, offset by `days` (negative = past)."""
    return int(time.time()) + days * 86400


# ──────────────────────────────────────────────────────────────────────────────
# Invoice / receipt templates. Each one carries its own (filename, size) so
# they line up with the attachment rules below — running the demo, the user can
# show how AWS / Google / Notion / Anthropic invoices automatically land in the
# Attachments view tagged by vendor.
# ──────────────────────────────────────────────────────────────────────────────

INVOICE_TEMPLATES_EN: list[tuple[str, str, str, str, str, int, str, int]] = [
    ("AWS Billing", "no-reply-aws@amazon.com",
     "Your AWS bill for {month} is now available",
     "Hello Alex,\n\nYour AWS bill for {month} 2026 is ${amount}. The PDF invoice is attached "
     "and the breakdown is available in the Billing Console.",
     "updates", 3, "aws-invoice-{month_short}.pdf", 184_231),
    ("Google Workspace", "workspace-noreply@google.com",
     "Your Google Workspace invoice for {month}",
     "Hello,\n\nAttached is your Google Workspace invoice for {month} 2026.\n\n"
     "Subtotal: $148.00\nTax: $13.32\nTotal: $161.32",
     "updates", 3, "google-workspace-{month_short}.pdf", 92_104),
    ("Notion Billing", "team@mail.notion.so",
     "Your Notion invoice for {month}",
     "Hi Alex,\n\nThanks for using Notion. Your invoice for {month} 2026 is attached.\n\n"
     "Plan: Business\nSeats: 18\nTotal: $144.00",
     "updates", 3, "notion-invoice-{month_short}.pdf", 64_512),
    ("Anthropic Billing", "billing@anthropic.com",
     "Anthropic receipt for {month}",
     "Thank you for using Anthropic. Your usage receipt for {month} 2026 is attached.\n\n"
     "API spend: $214.18",
     "updates", 2, "anthropic-receipt-{month_short}.pdf", 39_204),
    ("Linear", "billing@linear.app",
     "Linear invoice — {month} 2026",
     "Your Linear Business plan invoice for {month} 2026 is attached.\n\nSeats: 12\nTotal: $96.00",
     "updates", 2, "linear-invoice-{month_short}.pdf", 28_540),
    ("Stripe", "invoicing@stripe.com",
     "Stripe fees invoice — {month} 2026",
     "Your monthly Stripe processing fees invoice is attached.\n\nVolume: $42,818.20\nFees: $1,242.73",
     "updates", 2, "stripe-fees-{month_short}.pdf", 71_220),
]

INVOICE_TEMPLATES_ES: list[tuple[str, str, str, str, str, int, str, int]] = [
    ("AWS Billing", "no-reply-aws@amazon.com",
     "Tu factura de AWS de {month} ya está disponible",
     "Hola José,\n\nTu factura de AWS de {month} de 2026 es de {amount} €. La factura en PDF "
     "está adjunta y el desglose está disponible en la consola de facturación.",
     "updates", 3, "aws-factura-{month_short}.pdf", 184_231),
    ("Google Workspace", "workspace-noreply@google.com",
     "Tu factura de Google Workspace de {month}",
     "Hola,\n\nAdjuntamos tu factura de Google Workspace de {month} de 2026.\n\n"
     "Subtotal: 148,00 €\nIVA: 31,08 €\nTotal: 179,08 €",
     "updates", 3, "google-workspace-{month_short}.pdf", 92_104),
    ("Notion Billing", "team@mail.notion.so",
     "Tu factura de Notion de {month}",
     "Hola José,\n\nGracias por usar Notion. Adjuntamos tu factura de {month} de 2026.\n\n"
     "Plan: Business\nPuestos: 18\nTotal: 144,00 €",
     "updates", 3, "notion-factura-{month_short}.pdf", 64_512),
    ("Anthropic Billing", "billing@anthropic.com",
     "Recibo de Anthropic de {month}",
     "Gracias por usar Anthropic. Adjuntamos el recibo de uso de {month} de 2026.\n\n"
     "Consumo API: 214,18 €",
     "updates", 2, "anthropic-recibo-{month_short}.pdf", 39_204),
    ("Linear", "billing@linear.app",
     "Factura de Linear — {month} 2026",
     "Adjuntamos tu factura del plan Business de Linear de {month} de 2026.\n\n"
     "Puestos: 12\nTotal: 96,00 €",
     "updates", 2, "linear-factura-{month_short}.pdf", 28_540),
    ("Stripe", "invoicing@stripe.com",
     "Factura de tarifas Stripe — {month} 2026",
     "Adjuntamos la factura mensual de tarifas de procesamiento de Stripe.\n\n"
     "Volumen: 42.818,20 €\nTarifas: 1.242,73 €",
     "updates", 2, "stripe-tarifas-{month_short}.pdf", 71_220),
]

# Months used by the {month}/{month_short} substitution. Six entries so any
# vendor with weight≤3 gets a distinct month per instance — keeps the inbox
# from showing "AWS bill for March" three times in a row.
INVOICE_MONTHS_EN = ["January", "February", "March", "April", "May", "June"]
INVOICE_MONTHS_ES = ["enero", "febrero", "marzo", "abril", "mayo", "junio"]
INVOICE_AMOUNTS = ["1,847.12", "2,104.55", "1,612.40", "2,318.09", "1,950.27", "2,228.14"]
INVOICE_AMOUNTS_ES = ["1.847,12", "2.104,55", "1.612,40", "2.318,09", "1.950,27", "2.228,14"]


# Attachment rules — the demo shows that incoming invoices with PDFs are
# auto-classified into the Attachments view, grouped by vendor tag.
ATTACHMENT_RULES_EN: list[tuple[str, str | None, str | None, str | None, list[str]]] = [
    ("AWS Invoices",       "no-reply-aws@amazon.com",   "AWS bill",       "%.pdf", ["invoice", "aws"]),
    ("Google Workspace",   "workspace-noreply@google.com", "Google Workspace invoice", "%.pdf", ["invoice", "google"]),
    ("Notion Invoices",    "team@mail.notion.so",       "Notion invoice", "%.pdf", ["invoice", "notion"]),
    ("Anthropic Receipts", "billing@anthropic.com",     "Anthropic receipt", "%.pdf", ["invoice", "anthropic"]),
    ("Linear Invoices",    "billing@linear.app",        "Linear invoice", "%.pdf", ["invoice", "linear"]),
    ("Stripe Fees",        "invoicing@stripe.com",      "Stripe fees",    "%.pdf", ["invoice", "stripe"]),
]

ATTACHMENT_RULES_ES: list[tuple[str, str | None, str | None, str | None, list[str]]] = [
    ("Facturas AWS",           "no-reply-aws@amazon.com",      "factura de AWS",          "%.pdf", ["factura", "aws"]),
    ("Google Workspace",       "workspace-noreply@google.com", "factura de Google Workspace", "%.pdf", ["factura", "google"]),
    ("Facturas Notion",        "team@mail.notion.so",          "factura de Notion",       "%.pdf", ["factura", "notion"]),
    ("Recibos Anthropic",      "billing@anthropic.com",        "Recibo de Anthropic",     "%.pdf", ["factura", "anthropic"]),
    ("Facturas Linear",        "billing@linear.app",           "Factura de Linear",       "%.pdf", ["factura", "linear"]),
    ("Tarifas Stripe",         "invoicing@stripe.com",         "Factura de tarifas Stripe", "%.pdf", ["factura", "stripe"]),
]


LOCALE_EN = Locale(
    code="en",
    work=ACCOUNT_WORK,
    personal=ACCOUNT_PERSONAL,
    work_templates=WORK_TEMPLATES,
    personal_templates=PERSONAL_TEMPLATES,
    invoice_templates=INVOICE_TEMPLATES_EN,
    user_first_name="Alex",
    sent_subject_prefix="Re: ",
    reply_first=(
        "Thanks for flagging — taking a look now and will follow up later today.\n\n"
        "Best,\nAlex"
    ),
    reply_followup=(
        "Appreciate the quick response. Standing by — let me know if you need "
        "anything else from our side."
    ),
    sent_short="Thanks — looking into this and will get back to you shortly.\n\n— Alex",
    tasks=[
        ("Reply to Daniel re: enterprise plan",
         "Audit log + EU data residency questions for Helix Robotics",
         "high", "Helix Robotics", now_plus_days(2)),
        ("Approve Q2 pricing experiment",
         "Marco needs the scope locked before Thursday's planning",
         "high", "Northwind Labs", now_plus_days(1)),
        ("Decide on Lena's renewal counter (Orbit Freight)",
         "12% discount for 3y commit vs 2y flat",
         "normal", "Orbit Freight", now_plus_days(3)),
        ("Review PR #284 (Stripe webhook idempotency)",
         None, "normal", "Northwind Labs", None),
        ("Book annual physical", "Dr. Park reminder", "low", None, now_plus_days(14)),
    ],
    memory_facts=[
        ("contact", "daniel.osei@helixrobotics.com",
         "Daniel Osei is VP of Engineering at Helix Robotics, evaluating Northwind for a 60-seat rollout.",
         "Helix Robotics"),
        ("contact", "lena.park@orbitfreight.co",
         "Lena Park (Orbit Freight) is negotiating a multi-year renewal; legal will only sign 2y flat or 3y at 12% discount.",
         "Orbit Freight"),
        ("contact", "maya@brightpath.studio",
         "Maya Klein leads design at Brightpath Studio and is the primary champion for our product internally.",
         "Brightpath Studio"),
        ("domain", "northstar.vc",
         "Northstar VC led Northwind Labs' seed round; Jordan Wei is our partner contact.",
         "Northstar VC"),
        ("user", "self",
         "Alex Reyes is CEO/cofounder of Northwind Labs, a Series A SaaS company headquartered remote-first.",
         "Northwind Labs"),
        ("project", "q2-pricing",
         "Q2 pricing experiment is owned by Marco; scope needs to be locked before Thursday's planning.",
         "Northwind Labs"),
    ],
    attachments=[
        # Non-invoice attachments only — invoices are handled by invoice_templates.
        ("Senior eng candidate%", "Rafael_Mendes_CV.pdf",  312_054),
    ],
    attachment_rules=ATTACHMENT_RULES_EN,
    ai_output_language="English",
)

LOCALE_ES = Locale(
    code="es",
    work=ACCOUNT_WORK_ES,
    personal=ACCOUNT_PERSONAL_ES,
    work_templates=WORK_TEMPLATES_ES,
    personal_templates=PERSONAL_TEMPLATES_ES,
    invoice_templates=INVOICE_TEMPLATES_ES,
    user_first_name="José",
    sent_subject_prefix="Re: ",
    reply_first=(
        "Gracias por avisar — le echo un ojo ahora y te respondo más tarde.\n\n"
        "Un saludo,\nJosé"
    ),
    reply_followup=(
        "Gracias por la respuesta tan rápida. Quedo atento — avísame si necesitas "
        "algo más por nuestra parte."
    ),
    sent_short="Gracias — lo miro y te respondo en breve.\n\n— José",
    tasks=[
        ("Responder a Daniel sobre el plan enterprise",
         "Preguntas de audit log + residencia de datos EU para Helix Robotics",
         "high", "Helix Robotics", now_plus_days(2)),
        ("Aprobar el experimento de precios Q2",
         "Marco necesita que cerremos el alcance antes del planning del jueves",
         "high", "Viento Norte", now_plus_days(1)),
        ("Decidir contraoferta de renovación con Lena (Orbit Freight)",
         "12% de descuento por 3 años vs 2 años a la misma tarifa",
         "normal", "Orbit Freight", now_plus_days(3)),
        ("Revisar PR #284 (idempotencia de webhook de Stripe)",
         None, "normal", "Viento Norte", None),
        ("Reservar revisión médica anual", "Recordatorio de la doctora Park",
         "low", None, now_plus_days(14)),
    ],
    memory_facts=[
        ("contact", "daniel.osei@helixrobotics.com",
         "Daniel Osei es VP de Ingeniería en Helix Robotics y está evaluando Viento Norte para un despliegue de 60 puestos.",
         "Helix Robotics"),
        ("contact", "lena.park@orbitfreight.co",
         "Lena Park (Orbit Freight) está negociando una renovación plurianual; legal solo firma 2 años a la misma tarifa o 3 años con 12% de descuento.",
         "Orbit Freight"),
        ("contact", "maya@brightpath.studio",
         "Maya Klein lidera diseño en Brightpath Studio y es el principal embajador interno del producto.",
         "Brightpath Studio"),
        ("domain", "northstar.vc",
         "Northstar VC lideró la ronda Seed de Viento Norte; Jordan Wei es nuestro contacto en el partner.",
         "Northstar VC"),
        ("user", "self",
         "José Pérez es CEO/cofundador de Viento Norte, una SaaS en Serie A con sede remote-first.",
         "Viento Norte"),
        ("project", "precios-q2",
         "El experimento de precios Q2 lo lidera Marco; hay que cerrar el alcance antes del planning del jueves.",
         "Viento Norte"),
    ],
    attachments=[
        # Non-invoice attachments only — invoices are handled by invoice_templates.
        ("Candidato senior%",       "Rafael_Mendes_CV.pdf", 312_054),
    ],
    attachment_rules=ATTACHMENT_RULES_ES,
    ai_output_language="Spanish",
)


def get_locale(lang: str) -> Locale:
    if lang == "en":
        return LOCALE_EN
    if lang == "es":
        return LOCALE_ES
    raise SystemExit(f"unknown --lang '{lang}' (expected 'en' or 'es')")


# ──────────────────────────────────────────────────────────────────────────────
# Schema bootstrap
# ──────────────────────────────────────────────────────────────────────────────

VIRTUAL_TABLE_PREFIXES = ("emails_fts", "memory_facts_fts", "vec_emails", "vec_memory_facts")


def _is_shadow_table(name: str, virtual_tables: set[str]) -> bool:
    """FTS5 / vec0 auto-create shadow tables (e.g. emails_fts_data, vec_emails_chunks).
    These cannot be CREATE'd manually — they appear when the VIRTUAL TABLE is created."""
    if name in virtual_tables:
        return False
    return any(name.startswith(vt + "_") for vt in virtual_tables)


def copy_schema(prod_db: Path, demo_db: Path) -> None:
    if not prod_db.exists():
        raise SystemExit(
            f"prod DB not found at {prod_db}. "
            "Run the real app at least once so the schema exists, then retry."
        )
    demo_db.parent.mkdir(parents=True, exist_ok=True)
    if demo_db.exists():
        demo_db.unlink()
    for sidecar in (demo_db.with_suffix(".db-shm"), demo_db.with_suffix(".db-wal")):
        if sidecar.exists():
            sidecar.unlink()

    # Pull schema from sqlite_master so we can filter out:
    #   - sqlite_sequence (reserved name, auto-managed by SQLite)
    #   - FTS5 / vec0 shadow tables (auto-managed by the virtual table)
    src = sqlite3.connect(str(prod_db))
    try:
        rows = src.execute(
            """SELECT type, name, sql FROM sqlite_master
               WHERE sql IS NOT NULL
               ORDER BY CASE type
                   WHEN 'table' THEN 0
                   WHEN 'index' THEN 1
                   WHEN 'view' THEN 2
                   WHEN 'trigger' THEN 3
                   ELSE 4 END"""
        ).fetchall()
    finally:
        src.close()

    virtual_tables = {
        name for _t, name, sql in rows
        if sql and sql.lstrip().upper().startswith("CREATE VIRTUAL TABLE")
    }

    # vec0 isn't loaded in stock Python sqlite3, so we skip vec_* virtual tables
    # (and their shadow tables). The app re-creates them with IF NOT EXISTS on
    # startup once it loads the sqlite-vec extension.
    skip_virtual = {n for n in virtual_tables if n.startswith("vec_")}

    statements: list[str] = []
    for type_, name, sql in rows:
        if name.startswith("sqlite_"):
            continue
        if name in skip_virtual:
            continue
        if type_ == "table" and _is_shadow_table(name, virtual_tables):
            continue
        statements.append(sql.rstrip(";") + ";")

    # Also copy `refinery_schema_history` rows so that when the app (or eval
    # harness) reopens the demo DB and reruns embedded migrations they treat
    # every prod-applied version as already-applied. Without this, additive
    # ALTER-style migrations (which can't use `IF NOT EXISTS`) would fail on
    # the second run with "duplicate column" because the schema copy above
    # already brought the column over.
    src_history = sqlite3.connect(str(prod_db))
    try:
        history_rows = src_history.execute(
            "SELECT version, name, applied_on, checksum FROM refinery_schema_history ORDER BY version"
        ).fetchall()
    except sqlite3.OperationalError:
        history_rows = []  # Prod DB pre-dates refinery (very old install).
    finally:
        src_history.close()

    conn = sqlite3.connect(str(demo_db))
    try:
        conn.executescript("\n".join(statements))
        if history_rows:
            conn.executemany(
                "INSERT OR REPLACE INTO refinery_schema_history(version, name, applied_on, checksum) VALUES (?, ?, ?, ?)",
                history_rows,
            )
        conn.commit()
    finally:
        conn.close()


# ──────────────────────────────────────────────────────────────────────────────
# Data population
# ──────────────────────────────────────────────────────────────────────────────

def now_s() -> int:
    """Current Unix epoch in seconds.

    The app stores email timestamps as seconds (Gmail sync converts
    `internal_date` from millis to seconds before inserting), and every
    `created_at` / `email_timestamp` / `sync_from_timestamp` column in
    prod is on the same scale — so all demo rows must match.
    """
    return int(time.time())


def pick_timestamp_within_days(days: int) -> int:
    """Random Unix-seconds timestamp within the last `days` days."""
    now = now_s()
    delta = RNG.randint(0, days * 24 * 60 * 60)
    return now - delta


def insert_accounts(conn: sqlite3.Connection, locale: Locale) -> None:
    now = now_s()
    for i, acct in enumerate([locale.work, locale.personal]):
        conn.execute(
            """INSERT INTO accounts
               (id, provider, email, name, created_at, sort_order, enabled, sync_from_timestamp)
               VALUES (?, ?, ?, ?, ?, ?, 1, NULL)""",
            (acct.id, acct.provider, acct.email, acct.name, now - 90 * 86400, i),
        )
        conn.execute(
            """INSERT INTO sync_state
               (account_id, last_sync_at, history_id, next_page_token, status, error)
               VALUES (?, ?, NULL, NULL, 'idle', NULL)""",
            (acct.id, now - 3600),
        )
        # Empty category list = embed every category. The default ["primary"]
        # would skip most of our demo data (updates/promotions/social), so
        # widening this here is what makes `make demo-embed` cover the inbox.
        conn.execute(
            "INSERT OR REPLACE INTO user_preferences (key, value) VALUES (?, ?)",
            (f"embeddings_categories:{acct.id}", json.dumps([])),
        )


def insert_ai_config(conn: sqlite3.Connection) -> None:
    conn.execute(
        """INSERT OR REPLACE INTO ai_config
           (id, provider, model, api_key_id, monthly_budget_usd, period_start, is_active)
           VALUES (1, 'llamacpp', 'qwen3.5-4b-q4_k_m', NULL, 0.0, ?, 1)""",
        (now_s(),),
    )


PREFS: dict[str, str] = {
    "onboarding_completed": "true",
    "ai_enabled": "true",
    "ai_provider": "llamacpp",
    "ai_model": "qwen3.5-4b-q4_k_m",
    "ai_embedding_model": "nomic-embed-text-v1.5-q4_k_m",
    "ai_thinking_enabled": "false",
    "ai_monthly_budget": "0",
    "ai_output_language": "English",
    "classify_enabled": "true",
    "classify_provider": "llamacpp",
    "classify_model": "qwen3.5-4b-q4_k_m",
    "classify_intents": json.dumps([
        "request", "approval", "scheduling", "delivery", "question",
        "introduction", "feedback", "notification", "complaint",
        "promotion", "conversation",
    ]),
    "classify_topics": json.dumps([
        "billing", "contract", "project", "hiring", "support", "legal",
        "sales", "operations", "networking", "education", "finance",
        "travel", "personal", "marketing", "security",
    ]),
    "classify_categories": json.dumps(["primary"]),
    "inbox_categories": json.dumps(["primary"]),
    "inbox_layout": "full-width",
    "chat.routing_mode": "auto",
    # Include `updates` so chat can find Sentry alerts, Linear bug comments,
    # GitHub PR notifications, etc. — items a founder running these stacks
    # genuinely cares about when asked "what bugs were reported this month?".
    # The app-wide default is just `primary`; the demo widens it on purpose.
    "chat.default_categories": "primary,updates",
    "experimental.tasks_enabled": "true",
    "experimental.memories_enabled": "true",
    "task_enabled": "true",
    "memory_enabled": "true",
}


def insert_preferences(conn: sqlite3.Connection, locale: Locale) -> None:
    prefs = dict(PREFS)
    prefs["ai_output_language"] = locale.ai_output_language
    for k, v in prefs.items():
        conn.execute(
            "INSERT OR REPLACE INTO user_preferences (key, value) VALUES (?, ?)",
            (k, v),
        )


def make_email_html(plain_body: str, sender_name: str) -> str:
    safe = (plain_body
            .replace("&", "&amp;")
            .replace("<", "&lt;")
            .replace(">", "&gt;"))
    html_body = safe.replace("\n", "<br>")
    return (
        '<div style="font-family: -apple-system, BlinkMacSystemFont, sans-serif; '
        'font-size: 14px; line-height: 1.6; color: #1f2328;">'
        f"{html_body}"
        "</div>"
    )


def insert_email(
    conn: sqlite3.Connection,
    *,
    account: Account,
    sender_name: str,
    sender_email: str,
    subject: str,
    body: str,
    timestamp: int,
    is_read: bool,
    mailbox: str,
    category: str,
    thread_id: str | None = None,
) -> str:
    email_id = f"demo_{uuid.uuid4().hex[:16]}"
    thread_id = thread_id or f"thread_{uuid.uuid4().hex[:12]}"
    sender_domain = sender_email.split("@")[-1].lower()

    snippet = body.replace("\n", " ").strip()[:180]
    recipients_json = json.dumps([account.email])
    cc_json = "[]"
    now = now_s()
    html_body = make_email_html(body, sender_name)

    conn.execute(
        """INSERT INTO emails
           (id, account_id, thread_id, message_id, subject, sender, sender_email,
            sender_domain, recipients_json, cc_json, snippet, timestamp, is_read, is_deleted,
            triage_status, category, mailbox, raw_json, created_at)
           VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 0, NULL, ?, ?, NULL, ?)""",
        (
            email_id, account.id, thread_id,
            f"<{email_id}@demo>", subject, sender_name, sender_email,
            sender_domain, recipients_json, cc_json, snippet,
            timestamp, 1 if is_read else 0,
            category, mailbox,
            now,
        ),
    )

    conn.execute(
        "INSERT INTO email_bodies (email_id, body) VALUES (?, ?)",
        (email_id, html_body),
    )

    # FTS index — populated manually since the app uses external content.
    conn.execute(
        "INSERT INTO emails_fts (email_id, subject, sender, body) VALUES (?, ?, ?, ?)",
        (email_id, subject, f"{sender_name} <{sender_email}>", body),
    )

    return email_id


# Mirror of `PERSONAL_EMAIL_DOMAINS` in src-tauri/src/util/email_addr.rs — kept
# in sync so the demo's company labels behave identically to the prod app's
# `company_label_for`: corporate domains collapse to a stem (`acme.com` →
# `acme`), personal providers fall back to the individual address so
# `alice@gmail.com` stays distinct from `bob@gmail.com`.
PERSONAL_EMAIL_DOMAINS: frozenset[str] = frozenset({
    "gmail.com", "googlemail.com",
    "outlook.com", "hotmail.com", "live.com", "msn.com",
    "outlook.es", "outlook.fr", "outlook.de", "outlook.it",
    "outlook.com.br", "outlook.com.ar", "outlook.com.mx", "outlook.co.uk",
    "hotmail.es", "hotmail.fr", "hotmail.de", "hotmail.it",
    "hotmail.com.br", "hotmail.com.ar", "hotmail.com.mx", "hotmail.co.uk", "hotmail.co",
    "live.es", "live.fr", "live.de", "live.it",
    "live.com.mx", "live.com.ar", "live.co.uk",
    "yahoo.com", "yahoo.co.uk", "yahoo.es", "yahoo.fr", "yahoo.de", "yahoo.it",
    "yahoo.com.br", "yahoo.com.ar", "yahoo.com.mx", "yahoo.ca", "yahoo.com.au",
    "ymail.com", "rocketmail.com",
    "icloud.com", "me.com", "mac.com",
    "proton.me", "protonmail.com", "pm.me",
    "aol.com",
    "gmx.com", "gmx.de", "gmx.net", "gmx.es", "gmx.fr", "gmx.at", "gmx.ch", "gmx.co.uk",
    "mail.com", "fastmail.com", "fastmail.fm", "zoho.com",
    "tutanota.com", "tutamail.com", "tuta.io",
    "telefonica.net", "movistar.es", "terra.es", "ya.com",
})


def company_label_for(sender_email: str) -> str:
    """Port of `src-tauri/src/util/email_addr.rs::company_label_for`.
    Personal-domain → full lowercased address. Corporate → strip rightmost
    TLD label."""
    addr = sender_email.strip().lower()
    if "@" not in addr:
        return addr
    _, domain = addr.rsplit("@", 1)
    domain = domain.strip(".").strip()
    if domain in PERSONAL_EMAIL_DOMAINS:
        return addr
    stem, _, _ = domain.rpartition(".")
    return stem if stem else domain


def insert_tags(conn: sqlite3.Connection, email_id: str, subject: str, body: str, sender_email: str) -> None:
    now = now_s()
    intent = infer_intent(subject, body)
    topic = infer_topic(sender_email)
    company = company_label_for(sender_email)
    rows: list[tuple[str, str]] = [("intent", intent), ("topic", topic)]
    if company:
        rows.append(("company", company))
    for tag_type, tag_value in rows:
        conn.execute(
            """INSERT OR IGNORE INTO email_tags
               (email_id, tag_type, tag_value, confidence, created_at)
               VALUES (?, ?, ?, ?, ?)""",
            (email_id, tag_type, tag_value, 0.85, now),
        )


def _vary(template_subject: str, template_body: str, instance: int, locale_code: str) -> tuple[str, str]:
    """Inject per-instance variation so a template with weight>1 doesn't produce
    N identical-looking emails. We splice small numeric tokens into a few
    well-known formats; anything else gets a discreet ` · #N` suffix on the
    second-and-later instance so it still reads natural."""
    if instance == 0:
        return template_subject, template_body
    s, b = template_subject, template_body
    # Linear ticket numbers — bump the suffix
    if "[NWL-" in s or "[VTN-" in s:
        s = re.sub(r"-(\d+)\]", lambda m: f"-{int(m.group(1)) + 13 * instance}]", s, count=1)
    elif "PR #" in s:
        s = re.sub(r"PR #(\d+)", lambda m: f"PR #{int(m.group(1)) + 7 * instance}", s, count=1)
    elif "issue:" in s.lower() or "incidencia:" in s.lower():
        suffix = f" (visto {3 + instance * 2} veces)" if locale_code == "es" else f" (seen {3 + instance * 2} times)"
        s = s + suffix
    elif "summary" in s.lower() or "resumen" in s.lower():
        week_n = 18 - instance  # rolling weeks back
        suffix = f" — semana {week_n}" if locale_code == "es" else f" — week {week_n}"
        s = s + suffix
    elif "messages in" in s.lower() or "mensajes en" in s.lower():
        s = re.sub(r"\b\d+\b", str(2 + instance * 2), s, count=1)
    elif "candidate" in s.lower() or "candidato" in s.lower():
        # Different recruiter intros mention different candidates
        names_en = ["Rafael Mendes", "Priya Iyer", "Tomás Lacroix"]
        names_es = ["Rafael Mendes", "Priya Iyer", "Tomás Lacroix"]
        names = names_es if locale_code == "es" else names_en
        new_name = names[instance % len(names)]
        b = b.replace("Rafael Mendes", new_name)
    else:
        # Generic disambiguator on second-and-later instances
        s = f"{s} · #{instance + 1}"
    return s, b


def expand_templates(
    templates: list[tuple[str, str, str, str, str, int]],
    locale_code: str,
) -> list[tuple[str, str, str, str, str]]:
    """Flatten templates into concrete (sender_name, sender_email, subject,
    body, category) tuples, honoring `weight` as a hard cap and applying
    per-instance variation so repeated templates don't render as duplicates."""
    out: list[tuple[str, str, str, str, str]] = []
    for sender_name, sender_email, subject, body, category, weight in templates:
        for i in range(weight):
            s, b = _vary(subject, body, i, locale_code)
            out.append((sender_name, sender_email, s, b, category))
    RNG.shuffle(out)
    return out


def expand_invoices(
    templates: list[tuple[str, str, str, str, str, int, str, int]],
    locale_code: str,
) -> list[tuple[str, str, str, str, str, str, int]]:
    """Like `expand_templates`, but substitutes `{month}`/`{month_short}`/
    `{amount}` per instance and returns the attachment (filename, size) along
    with the email tuple so the caller can wire it into `attachments`."""
    months = INVOICE_MONTHS_ES if locale_code == "es" else INVOICE_MONTHS_EN
    amounts = INVOICE_AMOUNTS_ES if locale_code == "es" else INVOICE_AMOUNTS
    out: list[tuple[str, str, str, str, str, str, int]] = []
    for sender_name, sender_email, subject, body, category, weight, filename, size in templates:
        for i in range(weight):
            month = months[i % len(months)]
            month_short = month[:3].lower()
            amount = amounts[i % len(amounts)]
            ctx = {"month": month, "month_short": month_short, "amount": amount}
            s = subject.format(**ctx)
            b = body.format(**ctx)
            fn = filename.format(**ctx)
            out.append((sender_name, sender_email, s, b, category, fn, size))
    RNG.shuffle(out)
    return out


def populate_emails(conn: sqlite3.Connection, locale: Locale) -> list[str]:
    """Generate emails for both accounts using each template at most `weight`
    times. Returns the inserted email IDs.

    Invoice emails are handled by `populate_invoice_emails` so they get their
    attachment metadata wired up — keep them out of this loop to avoid the
    random thread-expansion / sent-mailbox swap that would garble them."""
    inserted_ids: list[str] = []

    plan: list[tuple[Account, list[tuple[str, str, str, str, str]]]] = [
        (locale.work, expand_templates(locale.work_templates, locale.code)),
        (locale.personal, expand_templates(locale.personal_templates, locale.code)),
    ]

    for account, items in plan:
        for sender_name, sender_email, subject, body, category in items:
            # Most emails are inbox; spice in a few sent / spam / trash.
            mailbox_roll = RNG.random()
            if mailbox_roll < 0.85:
                mailbox = "inbox"
            elif mailbox_roll < 0.95:
                mailbox = "sent"
                # swap sender/recipient feel: pretend the user sent it
                sender_name = account.name
                sender_email = account.email
                subject = locale.sent_subject_prefix + subject
                body = locale.sent_short
            elif mailbox_roll < 0.98:
                mailbox = "spam"
            else:
                mailbox = "trash"

            timestamp = pick_timestamp_within_days(45)
            is_read = RNG.random() < 0.70 or mailbox != "inbox"

            # Occasionally turn one into a small thread (2-3 messages).
            thread_size = 1
            if RNG.random() < 0.18 and mailbox == "inbox":
                thread_size = RNG.choice([2, 2, 3])

            base_thread_id = f"thread_{uuid.uuid4().hex[:12]}"
            for i in range(thread_size):
                # Subsequent thread messages alternate sender (incoming/outgoing).
                if i == 0:
                    s_name, s_email = sender_name, sender_email
                    s_subject, s_body = subject, body
                    ts = timestamp
                    read = is_read
                else:
                    if i % 2 == 1:
                        s_name = account.name
                        s_email = account.email
                        s_body = locale.reply_first
                    else:
                        s_name = sender_name
                        s_email = sender_email
                        s_body = locale.reply_followup
                    s_subject = subject if subject.lower().startswith("re:") else f"{locale.sent_subject_prefix}{subject}"
                    ts = timestamp + i * 3600 * RNG.randint(2, 36)
                    read = True if i % 2 == 1 else RNG.random() < 0.5

                email_id = insert_email(
                    conn,
                    account=account,
                    sender_name=s_name,
                    sender_email=s_email,
                    subject=s_subject,
                    body=s_body,
                    timestamp=ts,
                    is_read=read,
                    mailbox=mailbox,
                    category=category,
                    thread_id=base_thread_id,
                )
                insert_tags(conn, email_id, s_subject, s_body, s_email)
                inserted_ids.append(email_id)

    return inserted_ids


def insert_attachment_rules(conn: sqlite3.Connection, locale: Locale) -> dict[str, str]:
    """Insert attachment_rules for the work account and return a map from the
    rule's `sender_email_pattern` -> rule_id so the invoice inserter can wire
    matched attachments to the correct rule."""
    now = now_s()
    by_sender: dict[str, str] = {}
    for name, sender_pat, subject_pat, filename_pat, tags in locale.attachment_rules:
        rule_id = f"rule_{uuid.uuid4().hex[:12]}"
        conn.execute(
            """INSERT INTO attachment_rules
               (id, account_id, name, sender_email_pattern, subject_pattern,
                filename_pattern, tags_json, enabled, created_at, updated_at)
               VALUES (?, ?, ?, ?, ?, ?, ?, 1, ?, ?)""",
            (
                rule_id,
                locale.work.id,
                name,
                sender_pat,
                subject_pat,
                filename_pat,
                json.dumps(tags),
                now,
                now,
            ),
        )
        if sender_pat:
            by_sender[sender_pat] = rule_id
    return by_sender


# ──────────────────────────────────────────────────────────────────────────────
# Mock invoice PDFs.
#
# The Rust backend resolves `attachments.file_path` against the app data dir
# (see services::attachments::safe_attachment_path) — so we mirror the same
# layout the prod app produces: `attachments/{account_id}/{uuid}.pdf`.
# A pure-Python writer keeps the demo script dependency-free.
# ──────────────────────────────────────────────────────────────────────────────


def _pdf_escape(s: str) -> bytes:
    """Escape `( ) \\` for PDF string literals and encode to WinAnsi (cp1252).
    Non-representable chars become `?` — accents/€ survive the round-trip."""
    out: list[bytes] = []
    for ch in s:
        if ch == "(":
            out.append(b"\\(")
        elif ch == ")":
            out.append(b"\\)")
        elif ch == "\\":
            out.append(b"\\\\")
        else:
            try:
                out.append(ch.encode("cp1252"))
            except UnicodeEncodeError:
                out.append(b"?")
    return b"".join(out)


def _build_invoice_pdf(
    vendor: str,
    header_lines: list[str],
    items: list[tuple[str, str]],
    total_label: str,
    total_value: str,
) -> bytes:
    """Build a one-page invoice-looking PDF that any viewer renders correctly.
    `header_lines` go right under the vendor; `items` is description+amount rows;
    `total_label`/`total_value` are the bold-ish bottom-row pair."""
    content: list[bytes] = [b"BT"]
    # Vendor (24pt)
    content += [b"/F1 24 Tf", b"72 740 Td", b"(" + _pdf_escape(vendor) + b") Tj"]
    # Header lines (11pt)
    content += [b"/F1 11 Tf", b"0 -28 Td"]
    for line in header_lines:
        content += [b"(" + _pdf_escape(line) + b") Tj", b"0 -14 Td"]
    # Spacer
    content += [b"0 -16 Td"]
    # Item rows (description left, amount right at +360pt offset)
    for desc, amount in items:
        content += [
            b"(" + _pdf_escape(desc) + b") Tj",
            b"360 0 Td",
            b"(" + _pdf_escape(amount) + b") Tj",
            b"-360 -16 Td",
        ]
    # Total (14pt)
    content += [
        b"/F1 14 Tf",
        b"0 -16 Td",
        b"(" + _pdf_escape(total_label) + b") Tj",
        b"360 0 Td",
        b"(" + _pdf_escape(total_value) + b") Tj",
    ]
    content += [b"ET"]
    stream = b"\n".join(content)

    objs: list[bytes] = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] "
        b"/Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>",
        b"<< /Length " + str(len(stream)).encode("ascii") + b" >>\nstream\n" + stream + b"\nendstream",
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
    ]

    out = bytearray()
    out += b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n"
    offsets: list[int] = []
    for i, body in enumerate(objs, start=1):
        offsets.append(len(out))
        out += f"{i} 0 obj\n".encode("ascii") + body + b"\nendobj\n"
    xref_off = len(out)
    out += f"xref\n0 {len(objs) + 1}\n".encode("ascii")
    out += b"0000000000 65535 f \n"
    for off in offsets:
        out += f"{off:010d} 00000 n \n".encode("ascii")
    out += b"trailer\n"
    out += f"<< /Size {len(objs) + 1} /Root 1 0 R >>\n".encode("ascii")
    out += f"startxref\n{xref_off}\n%%EOF\n".encode("ascii")
    return bytes(out)


# Per-vendor invoice payloads. Keyed by `sender_name`. Each entry is a function
# (month_label, locale_code) → (header_lines, items, total_label, total_value).
# We split EN/ES inside each lambda so the PDF stays locale-native.

def _invoice_payload(
    vendor: str,
    month_label: str,
    locale_code: str,
) -> tuple[str, list[str], list[tuple[str, str]], str, str]:
    """Return (display_vendor, header_lines, items, total_label, total_value)."""
    es = locale_code == "es"
    invoice_no = f"INV-2026-{uuid.uuid4().hex[:6].upper()}"
    period_lbl = "Período de facturación" if es else "Billing period"
    invoice_lbl = "Factura nº" if es else "Invoice #"
    issue_lbl = "Fecha de emisión" if es else "Issue date"
    bill_to_lbl = "Facturar a" if es else "Bill to"
    bill_to_val = "Viento Norte S.L. — jose@vientonorte.io" if es else "Northwind Labs Inc. — alex@northwindlabs.io"
    subtotal_lbl = "Subtotal" if es else "Subtotal"
    tax_lbl = "IVA (21%)" if es else "Tax (8.875%)"
    total_lbl = "Total" if es else "Total"
    cur = "€" if es else "$"

    header = [
        f"{invoice_lbl}: {invoice_no}",
        f"{issue_lbl}: 2026-05-14",
        f"{period_lbl}: {month_label} 2026",
        f"{bill_to_lbl}: {bill_to_val}",
    ]

    # Catalog of line items per vendor. Amounts are stable so PDFs reproduce.
    table = {
        "AWS Billing": {
            "es": ([
                ("EC2 — instancias compute (us-east-1)", f"1.214,30 {cur}"),
                ("S3 — almacenamiento estándar", f"248,17 {cur}"),
                ("CloudFront — transferencia de datos", f"312,55 {cur}"),
                ("RDS — Postgres multi-AZ", f"72,10 {cur}"),
            ], f"1.847,12 {cur}", f"387,89 {cur}", f"2.235,01 {cur}"),
            "en": ([
                ("EC2 — compute instances (us-east-1)", f"{cur}1,214.30"),
                ("S3 — standard storage", f"{cur}248.17"),
                ("CloudFront — data transfer", f"{cur}312.55"),
                ("RDS — Postgres multi-AZ", f"{cur}72.10"),
            ], f"{cur}1,847.12", f"{cur}163.93", f"{cur}2,011.05"),
        },
        "Google Workspace": {
            "es": ([
                ("Workspace Business Standard × 18 puestos", f"148,00 {cur}"),
            ], f"148,00 {cur}", f"31,08 {cur}", f"179,08 {cur}"),
            "en": ([
                ("Workspace Business Standard × 18 seats", f"{cur}252.00"),
            ], f"{cur}252.00", f"{cur}22.36", f"{cur}274.36"),
        },
        "Notion Billing": {
            "es": ([
                ("Plan Business × 18 miembros", f"144,00 {cur}"),
            ], f"144,00 {cur}", f"30,24 {cur}", f"174,24 {cur}"),
            "en": ([
                ("Business plan × 18 members", f"{cur}270.00"),
            ], f"{cur}270.00", f"{cur}23.96", f"{cur}293.96"),
        },
        "Anthropic Billing": {
            "es": ([
                ("Consumo API — Claude Sonnet", f"168,42 {cur}"),
                ("Consumo API — Claude Haiku", f"45,76 {cur}"),
            ], f"214,18 {cur}", f"44,98 {cur}", f"259,16 {cur}"),
            "en": ([
                ("API usage — Claude Sonnet", f"{cur}214.40"),
                ("API usage — Claude Haiku", f"{cur}58.20"),
            ], f"{cur}272.60", f"{cur}24.19", f"{cur}296.79"),
        },
        "Linear": {
            "es": ([
                ("Plan Business × 12 puestos", f"96,00 {cur}"),
            ], f"96,00 {cur}", f"20,16 {cur}", f"116,16 {cur}"),
            "en": ([
                ("Business plan × 12 seats", f"{cur}180.00"),
            ], f"{cur}180.00", f"{cur}15.98", f"{cur}195.98"),
        },
        "Stripe": {
            "es": ([
                ("Tarifas de procesamiento (1,4% + 0,25 €)", f"1.118,28 {cur}"),
                ("Radar para Fraude", f"124,45 {cur}"),
            ], f"1.242,73 {cur}", f"260,97 {cur}", f"1.503,70 {cur}"),
            "en": ([
                ("Processing fees (2.9% + $0.30)", f"{cur}1,118.28"),
                ("Radar fraud detection", f"{cur}124.45"),
            ], f"{cur}1,242.73", f"{cur}110.29", f"{cur}1,353.02"),
        },
    }

    entry = table.get(vendor, {}).get(locale_code)
    if entry is None:
        # Generic fallback — shouldn't trigger for shipped vendors.
        return vendor, header, [("Service charge", f"{cur}100,00")], total_lbl, f"{cur}100,00"

    items, subtotal_val, tax_val, total_val = entry
    items_with_summary = list(items) + [(subtotal_lbl, subtotal_val), (tax_lbl, tax_val)]
    return vendor, header, items_with_summary, total_lbl, total_val


def _write_invoice_pdf(
    demo_dir: Path,
    account_id: str,
    vendor: str,
    month_label: str,
    locale_code: str,
) -> tuple[str, int]:
    """Write a mock invoice PDF under `{demo_dir}/attachments/{account_id}/`.
    Returns `(relative_path, byte_size)` — the relative path is what we store
    in `attachments.file_path` (Rust resolves it against the data dir)."""
    display_vendor, header, items, total_lbl, total_val = _invoice_payload(
        vendor, month_label, locale_code
    )
    pdf = _build_invoice_pdf(display_vendor, header, items, total_lbl, total_val)
    file_id = uuid.uuid4().hex
    rel = f"attachments/{account_id}/{file_id}.pdf"
    abs_path = demo_dir / rel
    abs_path.parent.mkdir(parents=True, exist_ok=True)
    abs_path.write_bytes(pdf)
    return rel, len(pdf)


# Month names for the {month}/{month_short} substitution — used both in
# subject lines and in the PDF "Billing period" line.
_MONTH_LABELS_EN = {
    "jan": "January", "feb": "February", "mar": "March",
    "apr": "April", "may": "May", "jun": "June",
}
_MONTH_LABELS_ES = {
    "ene": "enero", "feb": "febrero", "mar": "marzo",
    "abr": "abril", "may": "mayo", "jun": "junio",
}


def _month_label_from_filename(filename: str, locale_code: str) -> str:
    """Extract the month token from e.g. `aws-factura-feb.pdf` and translate
    it back to a display label. Keeps the PDF's period line aligned with the
    subject's month."""
    table = _MONTH_LABELS_ES if locale_code == "es" else _MONTH_LABELS_EN
    stem = filename.rsplit(".", 1)[0].lower()
    for token, label in table.items():
        if stem.endswith(f"-{token}") or stem.endswith(f"_{token}") or token in stem.split("-"):
            return label
    return next(iter(table.values()))


def populate_invoice_emails(
    conn: sqlite3.Connection,
    locale: Locale,
    rule_ids_by_sender: dict[str, str],
    demo_dir: Path,
) -> list[str]:
    """Insert one inbox email per invoice template instance, plus its
    `email_attachment_meta` (so the inbox shows the paperclip) and an
    `attachments` row tagged with the matching rule (so the Attachments view
    groups them by vendor). For each invoice we also write a real mock PDF to
    disk so opening the attachment in the demo renders an actual invoice."""
    inserted: list[str] = []
    items = expand_invoices(locale.invoice_templates, locale.code)
    account = locale.work
    now = now_s()

    for sender_name, sender_email, subject, body, category, filename, _hinted_size in items:
        timestamp = pick_timestamp_within_days(45)
        # Invoices skew "already seen" — they're transactional notifications.
        is_read = RNG.random() < 0.85

        email_id = insert_email(
            conn,
            account=account,
            sender_name=sender_name,
            sender_email=sender_email,
            subject=subject,
            body=body,
            timestamp=timestamp,
            is_read=is_read,
            mailbox="inbox",
            category=category,
        )
        insert_tags(conn, email_id, subject, body, sender_email)
        inserted.append(email_id)

        # Generate the mock PDF on disk. file_path is relative to the data dir
        # — that's what the Rust resolver expects.
        month_label = _month_label_from_filename(filename, locale.code)
        rel_path, real_size = _write_invoice_pdf(
            demo_dir, account.id, sender_name, month_label, locale.code
        )

        # Paperclip metadata for the inbox list.
        att_meta_id = f"att_{uuid.uuid4().hex[:12]}"
        conn.execute(
            """INSERT INTO email_attachment_meta
               (id, email_id, account_id, provider_attachment_id, filename, mime_type,
                file_size, file_path, inline_data)
               VALUES (?, ?, ?, '', ?, 'application/pdf', ?, ?, NULL)""",
            (att_meta_id, email_id, account.id, filename, real_size, rel_path),
        )

        # Attachments-view row, tagged to the matching rule (if any).
        rule_id = rule_ids_by_sender.get(sender_email)
        if rule_id is not None:
            att_id = f"att_{uuid.uuid4().hex[:12]}"
            # Look up rule tags to copy onto the attachment.
            tags_row = conn.execute(
                "SELECT tags_json FROM attachment_rules WHERE id = ?",
                (rule_id,),
            ).fetchone()
            tags_json = tags_row[0] if tags_row else "[]"
            conn.execute(
                """INSERT INTO attachments
                   (id, account_id, email_id, rule_id, gmail_attachment_id, filename,
                    mime_type, file_size, file_path, tags_json, sender_email, subject,
                    email_timestamp, created_at)
                   VALUES (?, ?, ?, ?, '', ?, 'application/pdf', ?, ?, ?, ?, ?, ?, ?)""",
                (
                    att_id,
                    account.id,
                    email_id,
                    rule_id,
                    filename,
                    real_size,
                    rel_path,
                    tags_json,
                    sender_email,
                    subject,
                    timestamp,
                    now,
                ),
            )
    return inserted


def insert_attachments_meta(conn: sqlite3.Connection, locale: Locale) -> None:
    """Add a couple of attachment-meta rows so emails show paperclips in the UI.

    No actual files are written — these are metadata-only entries and the UI
    handles missing file_path / inline_data gracefully (shows the filename badge).
    """
    for subject_like, filename, size in locale.attachments:
        rows = conn.execute(
            """SELECT id, account_id FROM emails
               WHERE subject LIKE ? LIMIT 6""",
            (subject_like,),
        ).fetchall()
        for email_id, account_id in rows:
            att_id = f"att_{uuid.uuid4().hex[:12]}"
            conn.execute(
                """INSERT INTO email_attachment_meta
                   (id, email_id, account_id, provider_attachment_id, filename, mime_type,
                    file_size, file_path, inline_data)
                   VALUES (?, ?, ?, '', ?, 'application/pdf', ?, NULL, NULL)""",
                (att_id, email_id, account_id, filename, size),
            )


def insert_pending_tasks(conn: sqlite3.Connection, locale: Locale) -> None:
    """A few tasks so the Tasks panel has something to show during the demo."""
    now = now_s()
    work = locale.work.id
    candidate_rows = conn.execute(
        """SELECT id, thread_id FROM emails
           WHERE account_id = ? AND mailbox = 'inbox'
           ORDER BY timestamp DESC LIMIT 30"""
        , (work,)
    ).fetchall()

    for i, (title, detail, priority, company, due_at) in enumerate(locale.tasks):
        src_email_id, src_thread_id = (candidate_rows[i] if i < len(candidate_rows) else (None, None))
        conn.execute(
            """INSERT INTO pending_tasks
               (id, account_id, title, detail, source, source_email_id, source_thread_id,
                assignee, status, priority, due_at, completed_at, created_at, updated_at, company)
               VALUES (?, ?, ?, ?, 'extracted', ?, ?, 'me', 'open', ?, ?, NULL, ?, ?, ?)""",
            (
                f"task_{uuid.uuid4().hex[:12]}",
                work, title, detail, src_email_id, src_thread_id,
                priority, due_at, now, now, company,
            ),
        )


def insert_memory_facts(conn: sqlite3.Connection, locale: Locale) -> None:
    """A small set of promoted facts so the memory panel is non-empty."""
    now = now_s()
    for subject_kind, subject_key, fact, company in locale.memory_facts:
        conn.execute(
            """INSERT INTO memory_facts
               (id, account_id, subject_kind, subject_key, fact, source, source_email_id,
                confidence, score, status, last_used_at, created_at, updated_at,
                domain, vigency, company)
               VALUES (?, ?, ?, ?, ?, 'extraction', NULL, 0.9, 1.0, 'promoted',
                       ?, ?, ?, NULL, NULL, ?)""",
            (
                f"fact_{uuid.uuid4().hex[:12]}",
                locale.work.id, subject_kind, subject_key, fact,
                now - 86400, now, now, company,
            ),
        )


# ──────────────────────────────────────────────────────────────────────────────
# Entry point
# ──────────────────────────────────────────────────────────────────────────────

def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--prod-db", type=Path, default=DEFAULT_PROD_DB,
                        help=f"Schema source (default: {DEFAULT_PROD_DB})")
    parser.add_argument("--demo-db", type=Path, default=None,
                        help=f"Demo DB output path (default depends on --lang: "
                             f"{DEFAULT_DEMO_DB} for en, {DEFAULT_DEMO_DB_ES} for es)")
    parser.add_argument("--lang", choices=["en", "es"], default="en",
                        help="Locale to generate (default: en)")
    args = parser.parse_args()

    locale = get_locale(args.lang)
    demo_db = args.demo_db or (DEFAULT_DEMO_DB if args.lang == "en" else DEFAULT_DEMO_DB_ES)
    demo_dir = demo_db.parent

    print(f"[demo-db] lang:          {args.lang}")
    print(f"[demo-db] schema source: {args.prod_db}")
    print(f"[demo-db] writing to:    {demo_db}")

    copy_schema(args.prod_db, demo_db)

    # Wipe the per-account attachments tree so re-running the script doesn't
    # leave orphan PDFs from previous invocations.
    att_root = demo_dir / "attachments"
    if att_root.exists():
        shutil.rmtree(att_root)

    conn = sqlite3.connect(str(demo_db))
    conn.execute("PRAGMA foreign_keys = ON")
    try:
        with conn:
            insert_accounts(conn, locale)
            insert_ai_config(conn)
            insert_preferences(conn, locale)
            rule_ids = insert_attachment_rules(conn, locale)
            populate_emails(conn, locale)
            populate_invoice_emails(conn, locale, rule_ids, demo_dir)
            insert_attachments_meta(conn, locale)
            insert_pending_tasks(conn, locale)
            insert_memory_facts(conn, locale)
        n = conn.execute("SELECT COUNT(*) FROM emails").fetchone()[0]
        threads = conn.execute("SELECT COUNT(DISTINCT thread_id) FROM emails").fetchone()[0]
        print(f"[demo-db] inserted {n} emails across {threads} threads")
        print(f"[demo-db] accounts: {[locale.work.email, locale.personal.email]}")
    finally:
        conn.close()

    print(f"\n[demo-db] done. Launch with:")
    print(f"  make demo" if args.lang == "en" else f"  make demo-es")
    return 0


if __name__ == "__main__":
    sys.exit(main())
