// A scripted OpenAI-compatible provider for trying Pipkin against a real Pi engine, offline.
// Every prompt makes the model write notes.txt with the write tool, then say it did. A prompt
// containing the word "count" runs `seq 1 3000` with the bash tool instead (a long result). A prompt
// containing the word "slow" is held for 30 seconds first (or until the client gives up), so a
// run stays in flight long enough to steer, queue and stop it by hand.
// Usage: node scripts/stub-provider.mjs [port]   (default 18765)
import http from "node:http";

const port = Number(process.argv[2] ?? 18765);
let turn = 0;
const chunk = (delta, finish, usage) =>
  `data: ${JSON.stringify({
    id: "stub", object: "chat.completion.chunk", created: 0, model: "scripted",
    choices: [{ index: 0, delta, finish_reason: finish ?? null }],
    ...(usage ? { usage } : {}),
  })}\n\n`;
const usage = { prompt_tokens: 10, completion_tokens: 5, total_tokens: 15 };

http.createServer((req, res) => {
  let body = "";
  req.on("data", (d) => (body += d));
  req.on("end", async () => {
    let messages = [];
    try { messages = JSON.parse(body).messages ?? []; } catch {}
    const afterTool = messages.at(-1)?.role === "tool";
    const last = messages.findLast((m) => m.role === "user");
    const text = typeof last?.content === "string" ? last.content : JSON.stringify(last?.content ?? "");
    const hold = !afterTool && /\bslow\b/i.test(text) ? 30000 : 0;
    let gone = false;
    res.on("close", () => (gone = true));
    if (hold) await new Promise((resolve) => { const t = setTimeout(resolve, hold); res.on("close", () => { clearTimeout(t); resolve(); }); });
    if (gone) return;
    res.writeHead(200, { "content-type": "text/event-stream", "cache-control": "no-cache" });
    res.write(chunk({ role: "assistant", content: "" }));
    if (afterTool) {
      res.write(chunk({ content: "Done: I wrote notes.txt." }));
      res.write(chunk({}, "stop", usage));
    } else {
      turn += 1;
      const count = /\bcount\b/i.test(text);
      res.write(chunk({ content: count ? "Counting. " : "Writing the notes. " }));
      res.write(chunk({ tool_calls: [{ index: 0, id: `call_${turn}`, type: "function", function: count ? {
        name: "bash",
        arguments: JSON.stringify({ command: "seq 1 3000" }),
      } : {
        name: "write",
        arguments: JSON.stringify({ path: "notes.txt", content: `stub run ${turn}\nhello from the stub\n` }),
      } }] }));
      res.write(chunk({}, "tool_calls", usage));
    }
    res.end("data: [DONE]\n\n");
  });
}).listen(port, "127.0.0.1", () => console.error(`stub provider on 127.0.0.1:${port}`));
