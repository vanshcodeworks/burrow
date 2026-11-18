const http = require('http');

const port = Number(process.env.PORT) || 3000;
const natsUrl = process.env.NATS_URL || '(not set)';

const server = http.createServer((req, res) => {
  if (req.url === '/health') {
    res.writeHead(200, { 'content-type': 'application/json' });
    res.end(JSON.stringify({ ok: true, nats: natsUrl }));
    return;
  }
  res.writeHead(200, { 'content-type': 'application/json' });
  res.end(JSON.stringify({ message: 'hello from Burrow', nats: natsUrl }));
});

server.listen(port, '127.0.0.1', () => {
  console.log(`api listening on 127.0.0.1:${port}`);
});
