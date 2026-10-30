/**
 * /updates: the signup form. Ported from the SvelteKit route (src/routes/updates/+page.server.ts):
 * same validation, Turnstile check, rate limits, D1 statements and metadata columns.
 *
 * GET  serves dist's /updates page with the Turnstile widget and the visitor's Referer filled in.
 * POST stores the signup. With `Accept: application/json` (client/updates.ts) it answers {ok: true} or
 *      {error}; without JS it serves the page again, with the error shown or the form swapped for the
 *      thank-you note.
 *
 * Everything put into the page goes through HTMLRewriter's setAttribute / setInnerContent (text mode).
 */
import type { Env } from './env';
import { boundedBody, csp, json, methodNotAllowed, plain, sameOrigin } from './http';

const EMAIL_MAX = 254;
const COMPANY_MAX = 200;
const WHY_MAX = 2000;
const META_MAX = 512;
/** A form post larger than this (bytes) is refused unread: the fields above plus a Turnstile token fit well inside. */
const REQUEST_MAX = 64 * 1024;
// Rate limit, counted over the last minute of signups in D1 (no IPs are kept, so the second cap is global).
const PER_EMAIL_PER_MINUTE = 3;
const ALL_PER_MINUTE = 60;
const EMAIL_RE = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;
const UTM = ['utm_source', 'utm_medium', 'utm_campaign', 'utm_term', 'utm_content'] as const;
const TURNSTILE_JS = 'https://challenges.cloudflare.com/turnstile/v0/api.js';

type Values = { email: string; company: string; why: string; ref: string };
type Outcome = { ok: true } | { error: string; status: number; values: Values };

export async function handleUpdates(request: Request, env: Env, url: URL): Promise<Response> {
	if (request.method === 'GET' || request.method === 'HEAD') {
		return page(env, url, { ref: request.headers.get('referer') ?? '' });
	}
	if (request.method !== 'POST') return methodNotAllowed('GET, POST');

	const wantsJson = (request.headers.get('accept') ?? '').includes('application/json');
	// SvelteKit refused cross-site form posts; keep that.
	if (!sameOrigin(request)) {
		return wantsJson ? json({ error: 'Cross-site request refused.' }, 403) : plain('Cross-site request refused.', 403);
	}
	const result = await signup(request, env, url);
	if (wantsJson) return 'error' in result ? json({ error: result.error }, result.status) : json({ ok: true });
	if ('error' in result) return page(env, url, { ...result.values, error: result.error }, result.status);
	return page(env, url, { ref: '', sent: true });
}

function text(v: unknown, max: number): string | null {
	if (typeof v !== 'string') return null;
	const t = v.trim();
	return t ? t.slice(0, max) : null;
}

async function turnstileOk(secret: string, token: string, ip: string | null): Promise<boolean> {
	const body = new FormData();
	body.append('secret', secret);
	body.append('response', token);
	if (ip) body.append('remoteip', ip); // sent to Turnstile only, never stored
	const res = await fetch('https://challenges.cloudflare.com/turnstile/v0/siteverify', { method: 'POST', body });
	if (!res.ok) return false;
	const out: { success?: boolean } = await res.json();
	return out.success === true;
}

async function signup(request: Request, env: Env, url: URL): Promise<Outcome> {
	const empty: Values = { email: '', company: '', why: '', ref: '' };
	const limited = await boundedBody(request, REQUEST_MAX);
	if (!limited) return { error: 'That form is too large.', status: 413, values: empty };
	let data: FormData;
	try {
		data = await limited.formData();
	} catch {
		return { error: 'Enter a valid email address.', status: 400, values: empty };
	}
	const entry = data.get('email');
	const raw = typeof entry === 'string' ? entry : '';
	const email = raw.trim().toLowerCase();
	const company = text(data.get('company'), COMPANY_MAX);
	const why = text(data.get('why'), WHY_MAX);
	const refEntry = data.get('ref');
	const values: Values = {
		email: raw.slice(0, EMAIL_MAX + 64),
		company: company ?? '',
		why: why ?? '',
		ref: typeof refEntry === 'string' ? refEntry.slice(0, META_MAX) : '',
	};
	const fail = (status: number, error: string): Outcome => ({ error, status, values });

	if (!env.DB) return fail(500, 'Signups are not set up on this server.');

	if (!email || email.length > EMAIL_MAX || !EMAIL_RE.test(email)) return fail(400, 'Enter a valid email address.');

	if (env.TURNSTILE_SECRET_KEY) {
		const token = data.get('cf-turnstile-response');
		const ok =
			typeof token === 'string' &&
			token !== '' &&
			(await turnstileOk(env.TURNSTILE_SECRET_KEY, token, request.headers.get('cf-connecting-ip')));
		if (!ok) return fail(400, 'The check did not pass. Try again.');
	}

	// The attempt is logged and the limit checked in one statement, so parallel posts cannot all pass a count read
	// before any of them is written. Only posts that got this far (valid email, Turnstile passed) are logged: logging
	// ones that failed the check would let anyone fill the global cap without solving Turnstile.
	const now = Math.floor(Date.now() / 1000);
	const admitted = await env.DB.prepare(
		`INSERT INTO signup_attempts (email, at)
		SELECT ?1, ?2
		WHERE (SELECT count(*) FROM signup_attempts WHERE email = ?1 AND at > ?3) < ?4
			AND (SELECT count(*) FROM signup_attempts WHERE at > ?3) < ?5
		RETURNING at`,
	)
		.bind(email, now, now - 60, PER_EMAIL_PER_MINUTE, ALL_PER_MINUTE)
		.first<{ at: number }>();
	if (!admitted) return fail(429, 'Too many signups right now. Try again in a minute.');

	const cf = request.cf;
	const h = request.headers;
	const q = url.searchParams;
	const meta = {
		country: text(cf?.country, META_MAX),
		region: text(cf?.region, META_MAX),
		city: text(cf?.city, META_MAX),
		asn: typeof cf?.asn === 'number' ? cf.asn : null,
		as_organization: text(cf?.asOrganization, META_MAX),
		timezone: text(cf?.timezone, META_MAX),
		user_agent: text(h.get('user-agent'), META_MAX),
		referer: text(data.get('ref'), META_MAX) ?? text(h.get('referer'), META_MAX),
		accept_language: text(h.get('accept-language'), META_MAX),
		src: text(q.get('src'), META_MAX),
		...Object.fromEntries(UTM.map((k) => [k, text(q.get(k), META_MAX)])),
	};
	const cols = Object.keys(meta);

	// A repeat signup keeps its first-touch metadata and updates the answers it gave this time.
	await env.DB.batch([
		env.DB.prepare('DELETE FROM signup_attempts WHERE at < ?').bind(now - 86400),
		env.DB.prepare(
			`INSERT INTO signups (email, company, why, ${cols.join(', ')})
			VALUES (?, ?, ?, ${cols.map(() => '?').join(', ')})
			ON CONFLICT (email) DO UPDATE SET
				company = coalesce(excluded.company, signups.company),
				why = coalesce(excluded.why, signups.why),
				updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')`,
		).bind(email, company, why, ...Object.values(meta)),
	]);
	return { ok: true };
}

type Fill = { ref: string; email?: string; company?: string; why?: string; error?: string; sent?: boolean };

/** dist's /updates page with the site key, referrer, the visitor's values and the outcome filled in. */
async function page(env: Env, url: URL, fill: Fill, status = 200): Promise<Response> {
	const asset = await env.ASSETS.fetch(new Request(new URL('/updates', url), { method: 'GET' }));
	if (!asset.ok) return asset;

	// The key comes from wrangler.jsonc; it is checked anyway, since it goes into an attribute.
	const siteKey = /^[A-Za-z0-9_-]{1,128}$/.test(env.TURNSTILE_SITE_KEY ?? '') ? env.TURNSTILE_SITE_KEY : '';
	let rw = new HTMLRewriter().on('input[name="ref"]', {
		element: (el) => void el.setAttribute('value', fill.ref.slice(0, META_MAX)),
	});
	if (siteKey) {
		rw = rw
			.on('head', { element: (el) => void el.append(`<script src="${TURNSTILE_JS}" async defer></script>`, { html: true }) })
			// Appended content is not rewritten again, so the widget is added and then given its key below.
			.on('.turnstile-slot', {
				element: (el) => void el.append('<div class="cf-turnstile" data-theme="dark"></div>', { html: true }),
			});
	}
	if (fill.email !== undefined) {
		const { email, company = '', why = '' } = fill;
		rw = rw
			.on('form.signup input[name="email"]', { element: (el) => void el.setAttribute('value', email) })
			.on('form.signup input[name="company"]', { element: (el) => void el.setAttribute('value', company) })
			.on('form.signup textarea[name="why"]', { element: (el) => void el.setInnerContent(why) });
	}
	if (fill.error) {
		const error = fill.error;
		rw = rw.on('form.signup p.error', {
			element: (el) => {
				el.removeAttribute('hidden');
				el.setInnerContent(error);
			},
		});
	}
	if (fill.sent) {
		rw = rw
			.on('form.signup', { element: (el) => void el.setAttribute('hidden', '') })
			.on('div.sent', { element: (el) => void el.removeAttribute('hidden') });
	}

	const headers = new Headers(asset.headers);
	headers.set('cache-control', 'no-store');
	headers.set('content-security-policy', csp({ turnstile: !!siteKey }));
	headers.delete('etag');
	headers.delete('content-length');
	const out = rw.transform(new Response(asset.body, { status, headers }));
	return siteKey ? withSiteKey(out, siteKey) : out;
}

/** Second pass: the widget appended by the first pass gets its site key through setAttribute. */
function withSiteKey(res: Response, siteKey: string): Response {
	return new HTMLRewriter()
		.on('.turnstile-slot .cf-turnstile', { element: (el) => void el.setAttribute('data-sitekey', siteKey) })
		.transform(res);
}
