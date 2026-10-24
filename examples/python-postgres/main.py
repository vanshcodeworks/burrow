"""A small FastAPI app backed by PostgreSQL, managed by Burrow."""

from __future__ import annotations

import html
import os

import psycopg
from fastapi import FastAPI
from fastapi.responses import HTMLResponse, PlainTextResponse

DATABASE_URL = os.environ.get("DATABASE_URL")
if not DATABASE_URL:
    raise RuntimeError("DATABASE_URL is not set; start this service through `burrow up`.")

app = FastAPI(title="Burrow Python example")

PAGE = """<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>Burrow · FastAPI + PostgreSQL</title>
    <style>
      :root { color-scheme: light dark; }
      body { font: 16px/1.5 system-ui, sans-serif; max-width: 42rem; margin: 4rem auto; padding: 0 1rem; }
      code { background: color-mix(in srgb, currentColor 10%, transparent); padding: 0.1em 0.35em; border-radius: 0.25rem; }
      li { margin: 0.25rem 0; }
    </style>
  </head>
  <body>
    <h1>Burrow is running</h1>
    <p>
      This page came from a native Python process, and the rows below came
      from a native PostgreSQL process. Both were started by
      <code>burrow up</code>.
    </p>
    <h2>Notes</h2>
    <ul>{{notes}}</ul>
    <p><code>GET /api/notes</code> returns the same rows as JSON.</p>
  </body>
</html>
"""


def rows() -> list[tuple[int, str]]:
    with psycopg.connect(DATABASE_URL) as connection:
        with connection.cursor() as cursor:
            cursor.execute("SELECT id, body FROM notes ORDER BY id")
            return [(int(row[0]), str(row[1])) for row in cursor.fetchall()]


@app.get("/health", response_class=PlainTextResponse)
def health() -> str:
    with psycopg.connect(DATABASE_URL) as connection:
        with connection.cursor() as cursor:
            cursor.execute("SELECT 1")
            cursor.fetchone()
    return "ok\n"


@app.get("/api/notes")
def notes() -> list[dict[str, object]]:
    return [{"id": identifier, "body": body} for identifier, body in rows()]


@app.get("/", response_class=HTMLResponse)
def index() -> str:
    items = "".join(f"<li>{html.escape(body)}</li>" for _, body in rows())
    return PAGE.replace("{{notes}}", items)
