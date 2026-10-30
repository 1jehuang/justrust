/**
 * Inline comments on a discussable page (`main[data-doc]`). The worker renders everything (the Rust renderer, renderer/core/src/comments.rs):
 * each block's count button and thread, the forms, the `_page` discussion. Without JS that page works by itself
 * (threads open on :target, forms post and redirect). This script only enhances it:
 *
 * - buttons toggle their thread in place instead of jumping to it;
 * - posting and deleting go through the JSON API, then the affected thread is swapped for the worker's fresh
 *   rendering of it (one GET of the page), so the markup always comes from the one renderer;
 * - times show relative ("3 minutes ago");
 * - `#c-<id>` opens that comment's thread and highlights it; `#block-<id>` (where sign-in returns to) opens that
 *   block's thread and focuses its form.
 *
 * Nothing here builds HTML from strings: text goes in with textContent, markup comes parsed from the worker's page.
 */

const PAGE_THREAD = 'block-_page';

function el<K extends keyof HTMLElementTagNameMap>(tag: K, cls?: string, text?: string): HTMLElementTagNameMap[K] {
	const e = document.createElement(tag);
	if (cls) e.className = cls;
	if (text !== undefined) e.textContent = text;
	return e;
}

async function errorOf(res: Response): Promise<string> {
	try {
		const j: unknown = await res.json();
		if (j && typeof j === 'object' && 'error' in j && typeof j.error === 'string') return j.error;
	} catch {
		// not JSON
	}
	return res.status === 401 ? 'Sign in to comment.' : res.status === 429 ? 'Too many comments; try again in a minute.' : `Request failed (${res.status}).`;
}

const RTF = new Intl.RelativeTimeFormat('en', { numeric: 'auto' });
function ago(iso: string): string {
	const t = Date.parse(iso);
	if (Number.isNaN(t)) return '';
	const s = (t - Date.now()) / 1000;
	const steps: [number, Intl.RelativeTimeFormatUnit][] = [
		[60, 'second'],
		[3600, 'minute'],
		[86400, 'hour'],
		[86400 * 7, 'day'],
		[86400 * 30, 'week'],
		[86400 * 365, 'month'],
	];
	const sizes: Record<string, number> = { second: 1, minute: 60, hour: 3600, day: 86400, week: 86400 * 7, month: 86400 * 30, year: 86400 * 365 };
	for (const [limit, unit] of steps) if (Math.abs(s) < limit) return RTF.format(Math.round(s / sizes[unit]!), unit);
	return RTF.format(Math.round(s / sizes.year!), 'year');
}

function relativeTimes(root: ParentNode) {
	for (const t of root.querySelectorAll<HTMLTimeElement>('.c-item time[datetime]')) {
		const rel = ago(t.dateTime);
		if (!rel) continue;
		t.title = t.textContent ?? '';
		t.textContent = rel;
	}
}

function start(main: HTMLElement) {
	const threadOf = (id: string) => document.getElementById(id) as HTMLElement | null;
	const buttonOf = (threadId: string) => main.querySelector<HTMLAnchorElement>(`.c-btn[aria-controls="${CSS.escape(threadId)}"]`);

	function setOpen(thread: HTMLElement, open: boolean) {
		thread.hidden = !open;
		buttonOf(thread.id)?.setAttribute('aria-expanded', String(open));
	}

	function open(thread: HTMLElement, focus: boolean) {
		setOpen(thread, true);
		if (focus) thread.querySelector('textarea')?.focus({ preventScroll: false });
	}

	/** Swap a thread (and its button) for the worker's current rendering of it. */
	async function reload(threadId: string) {
		const res = await fetch(location.pathname, { headers: { Accept: 'text/html' }, cache: 'no-store', credentials: 'same-origin' });
		if (!res.ok) throw new Error(`Could not reload comments (${res.status}).`);
		const fresh = new DOMParser().parseFromString(await res.text(), 'text/html');
		const next = fresh.getElementById(threadId);
		const old = threadOf(threadId);
		if (!next || !old) throw new Error('Could not reload comments.');
		const nextButton = fresh.querySelector(`.c-btn[aria-controls="${CSS.escape(threadId)}"]`);
		const oldButton = buttonOf(threadId);
		const thread = document.adoptNode(next);
		relativeTimes(thread);
		old.replaceWith(thread);
		if (nextButton && oldButton) {
			const b = document.adoptNode(nextButton);
			b.setAttribute('role', 'button');
			oldButton.replaceWith(b);
		}
		setOpen(thread, true);
	}

	function mark(item: HTMLElement) {
		for (const old of main.querySelectorAll('.c-item.c-target')) old.classList.remove('c-target');
		item.classList.add('c-target');
		item.scrollIntoView({ block: 'center' });
	}

	async function post(form: HTMLFormElement) {
		const ta = form.querySelector('textarea');
		const button = form.querySelector<HTMLButtonElement>('button[type=submit]:not([form])');
		const err = form.querySelector<HTMLElement>('p.error');
		const thread = form.closest<HTMLElement>('.c-thread');
		const data = new FormData(form);
		const body = String(data.get('body') ?? '').trim();
		if (!ta || !button || !thread || !body || button.disabled) return;
		button.disabled = true;
		button.textContent = 'Posting…';
		if (err) err.hidden = true;
		try {
			// The form's action is the API's URL under the site's base (the renderer writes it).
			const res = await fetch(form.action, {
				method: 'POST',
				headers: { 'Content-Type': 'application/json', Accept: 'application/json' },
				body: JSON.stringify({ doc: data.get('doc'), block: data.get('block'), body }),
			});
			if (!res.ok) throw new Error(await errorOf(res));
			const j = (await res.json()) as { comment: { id: number } };
			await reload(thread.id);
			const item = document.getElementById(`c-${j.comment.id}`);
			if (item) mark(item);
		} catch (e) {
			if (err) {
				err.textContent = e instanceof Error ? e.message : String(e);
				err.hidden = false;
			}
			button.disabled = false;
			button.textContent = 'Post';
		}
	}

	async function remove(form: HTMLFormElement) {
		const m = /\/api\/comments\/([0-9]+)\/delete$/.exec(form.action);
		const thread = form.closest<HTMLElement>('.c-thread');
		const at = form.closest('.c-main');
		const del = form.querySelector('button');
		if (!m || !thread || !at || !del || del.disabled) return;
		if (!confirm('Delete this comment?')) return;
		del.disabled = true;
		at.querySelector(':scope > .error')?.remove();
		try {
			// The form posts to <base>/api/comments/<id>/delete; the JSON API is the same URL without /delete.
			const res = await fetch(form.action.slice(0, -'/delete'.length), { method: 'DELETE' });
			if (!res.ok) throw new Error(await errorOf(res));
			await reload(thread.id);
		} catch (e) {
			del.disabled = false;
			at.appendChild(el('p', 'error', e instanceof Error ? e.message : String(e)));
		}
	}

	function followHash() {
		let hash: string;
		try {
			hash = decodeURIComponent(location.hash.slice(1));
		} catch {
			return; // a malformed escape such as "#%E0": not one of ours
		}
		if (hash.startsWith('c-')) {
			const item = document.getElementById(hash);
			const thread = item?.closest<HTMLElement>('.c-thread');
			if (!item || !thread) return;
			open(thread, false);
			mark(item);
		} else if (hash.startsWith('block-')) {
			const thread = threadOf(hash);
			if (!thread?.classList.contains('c-thread')) return;
			open(thread, false);
			thread.scrollIntoView({ block: 'center' });
			thread.querySelector('textarea')?.focus({ preventScroll: true });
		}
	}

	// Take over: from here the threads are shown by `hidden`, not by :target (styles/site.css).
	main.classList.add('c-live');
	for (const b of main.querySelectorAll<HTMLAnchorElement>('.c-btn')) b.setAttribute('role', 'button');
	for (const t of main.querySelectorAll<HTMLElement>('.c-thread')) setOpen(t, t.id === PAGE_THREAD);
	relativeTimes(main);

	main.addEventListener('click', (e) => {
		const b = e.target instanceof Element ? e.target.closest<HTMLAnchorElement>('.c-btn') : null;
		const id = b?.getAttribute('aria-controls');
		const thread = id ? threadOf(id) : null;
		if (!thread) return;
		e.preventDefault();
		if (thread.hidden) open(thread, true);
		else setOpen(thread, false);
	});
	main.addEventListener('submit', (e) => {
		const f = e.target;
		if (!(f instanceof HTMLFormElement)) return;
		if (f.classList.contains('c-compose')) {
			e.preventDefault();
			void post(f);
		} else if (f.classList.contains('c-delform')) {
			e.preventDefault();
			void remove(f);
		}
	});
	main.addEventListener('keydown', (e) => {
		const ta = e.target;
		if (!(ta instanceof HTMLTextAreaElement) || !(e.key === 'Enter' && (e.metaKey || e.ctrlKey))) return;
		const f = ta.closest<HTMLFormElement>('form.c-compose');
		if (!f) return;
		e.preventDefault();
		void post(f);
	});
	window.addEventListener('hashchange', followHash);
	followHash();
}

function init() {
	const main = document.querySelector<HTMLElement>('main[data-doc]');
	if (main) start(main);
}

if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', init);
else init();

export {};
