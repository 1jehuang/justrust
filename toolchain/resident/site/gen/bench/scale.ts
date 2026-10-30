// Axis domains and ticks for the charts on /bench.

const finite = (values: (number | null | undefined)[]) => values.filter((v): v is number => typeof v === 'number' && Number.isFinite(v));

/** A log domain around the values, at least 4x wide so that it always has two ticks. */
export function logDomain(values: (number | null | undefined)[]): [number, number] | null {
	const positive = finite(values).filter((v) => v > 0);
	if (!positive.length) return null;
	let lo = Math.min(...positive) / 1.25;
	let hi = Math.max(...positive) * 1.25;
	if (hi / lo < 4) {
		const mid = Math.sqrt(lo * hi);
		lo = mid / 2;
		hi = mid * 2;
	}
	return [lo, hi];
}

/** The 1-2-5 values in the domain, thinned to at most five. */
export function logTicks([lo, hi]: [number, number]) {
	const at = (steps: number[]) => {
		const out: number[] = [];
		for (let e = Math.floor(Math.log10(lo)); e <= Math.ceil(Math.log10(hi)); e++) {
			for (const k of steps) {
				const v = Number((k * 10 ** e).toPrecision(6));
				if (v >= lo && v <= hi) out.push(v);
			}
		}
		return out;
	};
	const dense = at([1, 1.5, 2, 3, 5, 7]);
	if (at([1, 2, 5]).length < 3 && dense.length <= 6) return dense;
	for (const steps of [[1, 2, 5], [1, 3], [1]]) {
		const ticks = at(steps);
		if (ticks.length <= 5) return ticks;
	}
	const ticks = at([1]);
	return ticks.filter((_, i) => i % Math.ceil(ticks.length / 5) === 0);
}

function niceStep(rough: number) {
	const power = 10 ** Math.floor(Math.log10(rough));
	const f = rough / power;
	return (f <= 1 ? 1 : f <= 2 ? 2 : f <= 2.5 ? 2.5 : f <= 5 ? 5 : 10) * power;
}

/** From zero to a round number above the values. */
export function linearDomain(values: (number | null | undefined)[]): [number, number] | null {
	const all = finite(values);
	if (!all.length) return null;
	const top = Math.max(...all) * 1.05 || 1;
	const step = niceStep(top / 4);
	return [0, Math.ceil(top / step) * step];
}

export function linearTicks([lo, hi]: [number, number]) {
	const step = niceStep((hi - lo) / 4);
	const out: number[] = [];
	for (let v = lo; v <= hi + step / 1e6; v += step) out.push(Number(v.toPrecision(6)));
	return out;
}
