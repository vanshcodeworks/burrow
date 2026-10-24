// A deliberately small API: one query, one HTML page, one health check.
// `pg` is the only dependency; see README.md for the install step.
"use strict";

const http = require("node:http");
const fs = require("node:fs");
const path = require("node:path");
const { Pool } = require("pg");

const port = Number(process.env.PORT);
if (!port) {
  console.error("PORT is not set; start this service through `burrow up`.");
  process.exit(1);
}

const pool = new Pool({ connectionString: process.env.DATABASE_URL });
const page = fs.readFileSync(path.join(__dirname, "index.html"), "utf8");

function send(response, status, type, body) {
  response.writeHead(status, {
    "content-type": type,
    "cache-control": "no-store",
  });
  response.end(body);
}

const server = http.createServer(async (request, response) => {
  try {
    if (request.url === "/health") {
      await pool.query("SELECT 1");
      return send(response, 200, "text/plain; charset=utf-8", "ok\n");
    }

    if (request.url === "/api/todos") {
      const result = await pool.query("SELECT id, title FROM todos ORDER BY id");
      return send(response, 200, "application/json", JSON.stringify(result.rows));
    }

    if (request.url === "/" || request.url === "/index.html") {
      const result = await pool.query("SELECT id, title FROM todos ORDER BY id");
      const items = result.rows
        .map((row) => `<li>${escapeHtml(row.title)}</li>`)
        .join("");
      return send(response, 200, "text/html; charset=utf-8", page.replace("{{todos}}", items));
    }

    send(response, 404, "text/plain; charset=utf-8", "not found\n");
  } catch (error) {
    console.error(error);
    send(response, 500, "text/plain; charset=utf-8", "database error\n");
  }
});

function escapeHtml(value) {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;");
}

server.listen(port, "127.0.0.1", () => {
  console.log(`API listening on http://127.0.0.1:${port}`);
});

for (const signal of ["SIGINT", "SIGTERM"]) {
  process.on(signal, () => {
    server.close(() => pool.end().finally(() => process.exit(0)));
  });
}
