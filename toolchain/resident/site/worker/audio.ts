/**
 * /audio/*: the narration (audio/README.md). Manifests (`<doc>.<hash>.json`) are always static assets. The audio
 * files (`<sha>.webm`, content-addressed, so cached forever) come from the R2 bucket bound as AUDIO when there is one
 * (production, SITE_AUDIO_REMOTE=1 builds), else from the assets (dev and `wrangler dev`, files copied into dist/).
 *
 * Either way a Range request gets a 206: Safari will not play media from a server that answers ranges with 200.
 */
import type { Env } from './env';
import { assetHeaders, methodNotAllowed, plain } from './http';

const FILE = /^\/audio\/([0-9a-f]{16,64})\.webm$/;
const IMMUTABLE = 'public, max-age=31536000, immutable';

/** `bytes=a-b`, `bytes=a-` or `bytes=-n` against a length; null when absent or not one satisfiable range. */
function range(header: string | null, size: number): { start: number; end: number } | null | 'bad' {
	if (!header) return null;
	const m = /^bytes=(\d*)-(\d*)$/.exec(header.trim());
	if (!m || (m[1] === '' && m[2] === '')) return 'bad';
	let start: number;
	let end: number;
	if (m[1] === '') {
		start = Math.max(0, size - Number(m[2]));
		end = size - 1;
	} else {
		start = Number(m[1]);
		end = m[2] === '' ? size - 1 : Math.min(Number(m[2]), size - 1);
	}
	return start > end || start >= size ? 'bad' : { start, end };
}

function serve(bytes: ArrayBuffer, request: Request, etag: string): Response {
	const size = bytes.byteLength;
	const headers = new Headers({
		'content-type': 'audio/webm',
		'cache-control': IMMUTABLE,
		'accept-ranges': 'bytes',
		etag,
		'x-content-type-options': 'nosniff',
	});
	const r = range(request.headers.get('range'), size);
	if (r === 'bad') {
		headers.set('content-range', `bytes */${size}`);
		headers.set('cache-control', 'no-store');
		return new Response(null, { status: 416, headers });
	}
	if (request.headers.get('if-none-match') === etag && !r) return new Response(null, { status: 304, headers });
	if (!r) {
		headers.set('content-length', String(size));
		return new Response(request.method === 'HEAD' ? null : bytes, { status: 200, headers });
	}
	headers.set('content-range', `bytes ${r.start}-${r.end}/${size}`);
	headers.set('content-length', String(r.end - r.start + 1));
	return new Response(request.method === 'HEAD' ? null : bytes.slice(r.start, r.end + 1), { status: 206, headers });
}

export async function handleAudio(request: Request, env: Env, url: URL): Promise<Response> {
	if (request.method !== 'GET' && request.method !== 'HEAD') return methodNotAllowed('GET, HEAD');
	const m = FILE.exec(url.pathname);
	if (!m) return assetHeaders(await env.ASSETS.fetch(request));
	const etag = `"${m[1]}"`;
	// One spoken line each, up to a few hundred kilobytes (about 90 s at 32 kb/s), so they are read whole and sliced here.
	if (env.AUDIO) {
		const obj = await env.AUDIO.get(`${m[1]}.webm`);
		if (!obj) return plain('Not found.', 404);
		return serve(await obj.arrayBuffer(), request, etag);
	}
	const res = await env.ASSETS.fetch(new Request(url, { method: 'GET' }));
	if (!res.ok) return res;
	return serve(await res.arrayBuffer(), request, etag);
}
