// OverFilm 편집기로 넘길 장면 그림 그리기 (core/overshorts/export/overfilm.py 가 부른다).
//
//   node scripts/stage-export.mjs <job.json>
//
// job.json: {spec, publicDir, outDir, spans: [{id, start, end}], browser?, gl?, ignoreCertificateErrors?}
// spec 은 render_spec 에 layers: 'stage' 를 붙인 것 (제목·자막·출처·소리 없이 장면만, 투명 바탕).
// 구간마다 첫 프레임과 마지막 프레임을 PNG 로 그려 보고, 같으면 그 PNG 한 장으로 끝낸다 (글 머리·댓글 한 장).
// 다르면 (긴 사진·본문이 천천히 내려가는 장면, 원문 동영상, 썰 카드형) 그 구간만 H.264 영상으로 그린다.
// 마지막 줄에 결과 JSON: {spans: [{id, file, kind: 'image'|'video', start, end}], errors: [{id, error}]}
// 또는 실패하면 {error: 원인 한 줄}. 호출 스택 등 전체 기록은 stderr (Python 이 stage-export.log 로 남김).
import {bundle} from '@remotion/bundler';
import {renderMedia, renderStill, selectComposition} from '@remotion/renderer';
import {mkdirSync, readFileSync, rmSync} from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
/** 오류의 원인 한 줄 (Remotion 브라우저 오류는 호출 스택이 길게 붙는다: 스택은 stderr 로만). */
const cause = (err) => String(err?.message ?? err).split('\n').find((l) => l.trim()) ?? String(err);

async function main() {
  const job = JSON.parse(readFileSync(process.argv[2], 'utf-8'));
  const inputProps = {...job.spec, layers: 'stage'};
  const outDir = path.resolve(job.outDir);
  mkdirSync(outDir, {recursive: true});

  const chromiumOptions = {ignoreCertificateErrors: Boolean(job.ignoreCertificateErrors)};
  if (job.gl) chromiumOptions.gl = job.gl;
  const common = {inputProps, chromiumOptions, browserExecutable: job.browser || null, logLevel: 'error'};

  const serveUrl = await bundle({entryPoint: path.join(here, '..', 'src', 'index.ts'),
    publicDir: path.resolve(job.publicDir)});
  const composition = await selectComposition({serveUrl, id: 'Story', ...common});
  const fps = composition.fps;
  const last = composition.durationInFrames - 1;
  const clamp = (f) => Math.max(0, Math.min(last, f));

  const out = [];
  const errors = [];
  for (const span of job.spans.slice(0, 200)) {
    const a = clamp(Math.round(span.start * fps));
    const b = clamp(Math.max(a, Math.round(span.end * fps) - 1));
    const name = `stage-${String(span.id).padStart(3, '0')}`;
    const first = path.join(outDir, `${name}.png`);
    try {
      await renderStill({composition, serveUrl, output: first, frame: a, imageFormat: 'png', ...common});
    } catch (err) {
      // 한 장면을 못 그려도 나머지는 넘긴다 (편집기에서 그 자리만 비어 보임)
      console.error(err);
      errors.push({id: span.id, error: cause(err)});
      continue;
    }
    let still = true;
    if (b > a) {
      const lastPng = path.join(outDir, `${name}-end.png`);
      try {
        await renderStill({composition, serveUrl, output: lastPng, frame: b, imageFormat: 'png', ...common});
        // 같은 그림이면 PNG 바이트도 같다
        still = Buffer.compare(readFileSync(first), readFileSync(lastPng)) === 0;
      } catch (err) {
        console.error(err);
      }
      rmSync(lastPng, {force: true});
    }
    if (!still) {
      const video = path.join(outDir, `${name}.mp4`);
      try {
        await renderMedia({composition, serveUrl, codec: 'h264', outputLocation: video, frameRange: [a, b],
          muted: true, crf: 16, imageFormat: 'jpeg', ...common});
        rmSync(first, {force: true});
        out.push({id: span.id, file: path.basename(video), kind: 'video', start: a / fps, end: (b + 1) / fps});
        continue;
      } catch (err) {
        // 영상으로 못 그리면 첫 프레임 그림으로
        console.error(err);
        errors.push({id: span.id, error: `영상 대신 그림: ${cause(err)}`});
        rmSync(video, {force: true});
      }
    }
    // 바로 앞 구간과 같은 그림이면 (댓글이 이미 다 보이는 장 등) 앞 클립을 늘린다
    const prev = out[out.length - 1];
    const bytes = readFileSync(first);
    if (prev && prev.kind === 'image' && prev.bytes && Buffer.compare(prev.bytes, bytes) === 0
      && Math.abs(prev.end - a / fps) < 0.5 / fps) {
      prev.end = (b + 1) / fps;
      rmSync(first, {force: true});
      continue;
    }
    out.push({id: span.id, file: path.basename(first), kind: 'image', start: a / fps, end: (b + 1) / fps, bytes});
  }
  if (!out.length && errors.length) {
    // 하나도 못 그렸으면 실패로 (첫 원인)
    throw new Error(errors[0].error);
  }
  return {spans: out.map(({bytes: _bytes, ...rest}) => rest), errors};
}

try {
  console.log(JSON.stringify(await main()));
} catch (err) {
  console.error(err);
  console.log(JSON.stringify({error: cause(err)}));
  process.exitCode = 1;
}
