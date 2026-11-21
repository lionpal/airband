// A probe of Gemini's live voice API, run from a computer, to settle the facts before the phone is built on it:
// can a short-lived token be minted, does the model call our tools, how fast is the first audio, what does it say.
//   GEMINI_API_KEY=… node tools/live-probe.mjs [out.wav]
// It plays the part of the phone and of the tower: the "tower" here is two canned inbox messages.
import { writeFileSync } from 'node:fs';

const KEY = process.env.GEMINI_API_KEY;
if (!KEY) throw new Error('set GEMINI_API_KEY');
const MODEL = process.env.LIVE_MODEL || 'models/gemini-3.8-live';
const out = process.argv[2] || 'live-probe.wav';
const t0 = Date.now();
const at = () => `${((Date.now() - t0) / 1000).toFixed(2)}s`;

// 1. the tower mints a token the phone can use once; the API key never leaves the tower
const minted = await fetch('https://generativelanguage.googleapis.com/v1beta/auth_tokens', {
  method: 'POST',
  headers: { 'x-goog-api-key': KEY, 'content-type': 'application/json' },
  body: JSON.stringify({ uses: 1 }),
});
const token = await minted.json();
if (!minted.ok) throw new Error(`token: HTTP ${minted.status} ${JSON.stringify(token).slice(0, 300)}`);
console.log(at(), 'token minted; fields:', Object.keys(token).join(', '));

// 2. the phone opens the live session with that token
const inbox = [
  { id: 'a1', from: 'shop two', needs_you: true, text: 'Two questions are too easy. Rewrite them or leave them?' },
  { id: 'b2', from: 'backoffice one', needs_you: false, text: 'The Spanish reading lexicon is rebuilt.' },
];
const ws = new WebSocket(`wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContentConstrained?access_token=${encodeURIComponent(token.name)}`);
const pcm = [];
let firstAudio = null, said = '', answered = null, stage = 0;
const send = (o) => ws.send(JSON.stringify(o));

ws.onopen = () => {
  console.log(at(), 'socket open');
  send({ setup: {
    model: MODEL,
    generationConfig: { responseModalities: ['AUDIO'], speechConfig: { voiceConfig: { prebuiltVoiceConfig: { voiceName: 'Kore' } } } },
    outputAudioTranscription: {},
    systemInstruction: { parts: [{ text:
      'You are the voice of a radio channel between a man and his coding agents. Call next_message to get what is waiting. ' +
      'Read each message exactly as written, with the agent\'s name first, and nothing else: no greeting, no commentary. ' +
      'When he answers, call answer with the id of the message he is answering and his words, then say only "Sent to" and the agent\'s name.' }] },
    tools: [{ functionDeclarations: [
      { name: 'next_message', description: 'The next message waiting for him, or nothing.', parameters: { type: 'OBJECT', properties: {} } },
      { name: 'answer', description: 'Pass his answer to the agent that asked.', parameters: { type: 'OBJECT', properties: { id: { type: 'STRING' }, text: { type: 'STRING' } }, required: ['id', 'text'] } },
    ] }],
  } });
};
ws.onmessage = async (ev) => {
  const m = JSON.parse(typeof ev.data === 'string' ? ev.data : Buffer.from(await ev.data.arrayBuffer()).toString());
  if (m.setupComplete !== undefined) {
    console.log(at(), 'setup complete');
    send({ realtimeInput: { text: 'What is waiting for me?' } });
  }
  for (const call of m.toolCall?.functionCalls ?? []) {
    console.log(at(), 'tool call:', call.name, JSON.stringify(call.args ?? {}));
    let response = {};
    if (call.name === 'next_message') response = inbox.shift() ?? { nothing: true };
    if (call.name === 'answer') { answered = call.args; response = { ok: true }; }
    send({ toolResponse: { functionResponses: [{ id: call.id, name: call.name, response }] } });
  }
  const sc = m.serverContent;
  if (sc?.outputTranscription?.text) said += sc.outputTranscription.text;
  for (const p of sc?.modelTurn?.parts ?? []) if (p.inlineData?.data) {
    if (!firstAudio) { firstAudio = at(); console.log(firstAudio, 'first audio', p.inlineData.mimeType); }
    pcm.push(Buffer.from(p.inlineData.data, 'base64'));
  }
  // a turn that only called a tool also ends with turnComplete; the speech comes in the turn after our response
  if (sc?.turnComplete && said.trim()) {
    console.log(at(), `it said: "${said.trim()}"`);
    said = '';
    if (stage === 0) { stage = 1; firstAudio = null; console.log(at(), 'he says: "Rewrite them."'); send({ realtimeInput: { text: 'Rewrite them.' } }); }
    else { finish(); }
  }
  if (m.goAway) console.log(at(), 'goAway', JSON.stringify(m.goAway));
};
ws.onerror = (e) => console.log(at(), 'socket error', e.message ?? '');
ws.onclose = (e) => { console.log(at(), `socket closed ${e.code} ${e.reason}`); if (!done) finish(); };

let done = false;
function finish() {
  if (done) return;
  done = true;
  const data = Buffer.concat(pcm);
  const h = Buffer.alloc(44);
  h.write('RIFF', 0); h.writeUInt32LE(36 + data.length, 4); h.write('WAVEfmt ', 8); h.writeUInt32LE(16, 16); h.writeUInt16LE(1, 20); h.writeUInt16LE(1, 22);
  h.writeUInt32LE(24000, 24); h.writeUInt32LE(48000, 28); h.writeUInt16LE(2, 32); h.writeUInt16LE(16, 34); h.write('data', 36); h.writeUInt32LE(data.length, 40);
  writeFileSync(out, Buffer.concat([h, data]));
  console.log(at(), `${(data.length / 48000).toFixed(1)} s of speech saved to ${out}; answer passed on: ${JSON.stringify(answered)}`);
  try { ws.close(); } catch {}
  setTimeout(() => process.exit(0), 200);
}
setTimeout(() => { console.log(at(), 'gave up waiting'); finish(); }, 45000);
