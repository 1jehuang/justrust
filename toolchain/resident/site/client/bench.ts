/**
 * /bench (the BenchHistory widget, gen/bench/index.ts): redraws a part of the page from the embedded history when
 * a choice is clicked, or when the pointer or the arrow keys pick another run on a chart.
 */
import type { History } from '../gen/bench/data';
import { GROUP_PART, M, PARTS, PW, W, choose, initial } from '../gen/bench/view';

const root = document.querySelector<HTMLElement>('.bench');
const json = document.getElementById('bench-data')?.textContent;

if (root && json) {
	const data: History = JSON.parse(json);
	const state = initial();
	const part = (name: string) => root.querySelector<HTMLElement>(`[data-part="${name}"]`);

	function draw(names: (keyof typeof PARTS)[]) {
		// A redrawn chart is a new element: give focus back to the one that had it.
		const focused = document.activeElement;
		let restore: [string, number] | undefined;
		for (const name of names) {
			const el = part(name);
			if (!el) continue;
			const charts = [...el.querySelectorAll('svg[role=slider]')];
			if (focused && charts.includes(focused)) restore = [name, charts.indexOf(focused)];
			el.innerHTML = PARTS[name](data, state);
		}
		if (restore) part(restore[0])?.querySelectorAll<SVGSVGElement>('svg[role=slider]')[restore[1]]?.focus();
		if (focused instanceof HTMLButtonElement && focused.dataset.set && !focused.isConnected) {
			const same = [...root!.querySelectorAll<HTMLButtonElement>(`button[data-set="${focused.dataset.set}"]`)];
			same.find((b) => b.dataset.value === focused.dataset.value)?.focus();
		}
	}

	function pick(svg: SVGSVGElement, index: number) {
		const group = svg.dataset.group as keyof typeof GROUP_PART;
		const n = Number(svg.dataset.n);
		const i = Math.min(n - 1, Math.max(0, index));
		if (!(group in GROUP_PART) || i === Number(svg.dataset.active)) return;
		state[group] = i;
		draw([GROUP_PART[group]]);
	}

	root.addEventListener('click', (e) => {
		const button = (e.target as Element).closest<HTMLButtonElement>('button[data-set]');
		if (!button) return;
		choose(state, button.dataset.set!, button.dataset.value!);
		draw(['latest', 'time', 'live']);
	});

	const onPointer = (e: PointerEvent) => {
		const svg = (e.target as Element).closest<SVGSVGElement>('svg[data-group]');
		if (!svg) return;
		const n = Number(svg.dataset.n);
		const box = svg.getBoundingClientRect();
		const px = ((e.clientX - box.left) / box.width) * W;
		pick(svg, n <= 1 ? 0 : Math.round(((px - M.l) / PW) * (n - 1)));
	};
	root.addEventListener('pointermove', onPointer);
	root.addEventListener('pointerdown', onPointer);

	root.addEventListener('keydown', (e) => {
		const svg = (e.target as Element).closest<SVGSVGElement>('svg[data-group]');
		if (!svg) return;
		const active = Number(svg.dataset.active);
		const to = ({ ArrowLeft: active - 1, ArrowRight: active + 1, Home: 0, End: Number(svg.dataset.n) - 1 } as Record<string, number>)[e.key];
		if (to === undefined) return;
		e.preventDefault();
		pick(svg, to);
	});
}
