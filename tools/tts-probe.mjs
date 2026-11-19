// How fast is Gemini's speech model at reading one message? Streams it and times the first audio.
//   GEMINI_API_KEY=… node tools/tts-probe.mjs [model] [voice] [out.wav]
import { writeFileSync } from 'node:fs';
const [model = 'gemini-3.8-flash-lite-tts', voice = 'Leda', out] = process.argv.slice(2);
const text = 'Say exactly, in a calm clear voice: Shop two. Two questions are too easy. Rewrite them or leave them?';
const t0 = Date.now();
const res = await fetch(`https://generativelanguage.googleapis.com/v1beta/models/${model}:streamGenerateContent?alt=sse`, {
  method: 'POST',
  headers: { 'x-goog-api-key': process.env.GEMINI_API_KEY, 'content-type': 'application/json' },
  body: JSON.stringify({ contents: [{ parts: [{ text }] }], generationConfig: { responseModalities: ['AUDIO'], speechConfig: { voiceConfig: { prebuiltVoiceConfig: { voiceName: voice } } } } }),
});
if (!res.ok) { console.log(model, voice, 'HTTP', res.status, (await res.text()).slice(0, 200)); process.exit(1); }
let first = null, buf = '', pcm = [];
for await (const chunk of res.body) {
  buf += Buffer.from(chunk).toString();
  buf = buf.replace(/\r/g, '');
  let i;
  while ((i = buf.indexOf('\n\n')) >= 0) {
    const line = buf.slice(0, i).replace(/^data: /, ''); buf = buf.slice(i + 2);
    try {
      for (const p of JSON.parse(line).candidates?.[0]?.content?.parts ?? []) if (p.inlineData?.data) {
        if (first === null) first = Date.now() - t0;
        pcm.push(Buffer.from(p.inlineData.data, 'base64'));
      }
    } catch {}
  }
}
const data = Buffer.concat(pcm);
console.log(`${model} ${voice}: first audio ${(first / 1000).toFixed(2)}s, all ${((Date.now() - t0) / 1000).toFixed(2)}s, ${(data.length / 48000).toFixed(1)}s of speech`);
if (out) {
  const h = Buffer.alloc(44);
  h.write('RIFF', 0); h.writeUInt32LE(36 + data.length, 4); h.write('WAVEfmt ', 8); h.writeUInt32LE(16, 16); h.writeUInt16LE(1, 20); h.writeUInt16LE(1, 22);
  h.writeUInt32LE(24000, 24); h.writeUInt32LE(48000, 28); h.writeUInt16LE(2, 32); h.writeUInt16LE(16, 34); h.write('data', 36); h.writeUInt32LE(data.length, 40);
  writeFileSync(out, Buffer.concat([h, data]));
}
