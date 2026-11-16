// Peeby's `n` challenge server, appended to yt-dlp's EJS solver scripts (lib, then core): one
// JSON request per line on stdin, one JSON answer per line on stdout (crates/innertube/src/jsc.rs).
//
// Request: {"player": "<player JS path>", "code": "<player JS>" | null, "n": ["<challenge>", ...]}
// Answer:  {"ok": {"<challenge>": "<solved>", ...}} | {"need_player": true} | {"error": "..."}
//
// EJS's `jsc` turns a player JS into "preprocessed" code that sets `_result.n`; it is evaluated
// once per player and its function kept, so a challenge costs a call, not a parse of 2 MB.

const solvers = new Map();
const encoder = new TextEncoder();

function answer(request) {
  let solver = solvers.get(request.player);
  if (!solver && typeof request.code === "string") {
    const output = jsc({
      type: "player",
      player: request.code,
      requests: [],
      output_preprocessed: true,
    });
    if (output.type !== "result" || !output.preprocessed_player) {
      return { error: `preprocessing failed: ${output.error ?? output.type}` };
    }
    solver = { n: null, sig: null };
    Function("_result", output.preprocessed_player)(solver);
    solvers.set(request.player, solver);
  }
  if (!solver) {
    return { need_player: true };
  }
  if (typeof solver.n !== "function") {
    return { error: "no n function in this player" };
  }
  return { ok: Object.fromEntries(request.n.map((challenge) => [challenge, solver.n(challenge)])) };
}

function reply(value) {
  const bytes = encoder.encode(JSON.stringify(value) + "\n");
  let written = 0;
  while (written < bytes.length) {
    written += Deno.stdout.writeSync(bytes.subarray(written));
  }
}

const decoder = new TextDecoder();
let pending = "";
for await (const chunk of Deno.stdin.readable) {
  pending += decoder.decode(chunk, { stream: true });
  let end;
  while ((end = pending.indexOf("\n")) >= 0) {
    const line = pending.slice(0, end);
    pending = pending.slice(end + 1);
    if (!line.trim()) {
      continue;
    }
    let value;
    try {
      value = answer(JSON.parse(line));
    } catch (e) {
      value = { error: String(e instanceof Error ? e.message : e).slice(0, 300) };
    }
    reply(value);
  }
}
