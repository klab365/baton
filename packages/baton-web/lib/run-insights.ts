type Usage = {
	inputTokens?: number | null;
	outputTokens?: number | null;
	cacheReadTokens?: number | null;
	cacheWriteTokens?: number | null;
	estimatedCost?: string | null;
	currency?: string | null;
	priceUnit?: string | null;
	coverage?: string | null;
	inputPrice?: string | null;
	outputPrice?: string | null;
	cacheReadPrice?: string | null;
	cacheWritePrice?: string | null;
	priceSource?: string | null;
	priceCheckedAt?: string | null;
};
type RunStep = {
	stepId: string;
	attempt: number;
	status: string;
	durationMs?: number | null;
	usage?: Usage | null;
};
type Coverage = "complete" | "partial" | "unavailable";

type UsageSummary = {
	cost?: string | null;
	currency?: string | null;
	coverage?: string;
};

export function runInsights(
	allSteps: RunStep[],
	serverSummary?: UsageSummary | null,
) {
	const steps = allSteps.filter(
		(step) => step.status !== "pending" || step.attempt > 0,
	);
	const coverage = (count: number): Coverage =>
		count === 0
			? "unavailable"
			: count === steps.length
				? "complete"
				: "partial";
	const metric = (key: keyof Usage) => {
		const values = steps
			.map((step) => step.usage?.[key])
			.filter((value): value is number => typeof value === "number");
		return {
			value: values.length
				? values.reduce((sum, value) => sum + value, 0)
				: null,
			coverage: coverage(values.length),
		};
	};
	const durations = steps
		.map((step) => step.durationMs)
		.filter((value): value is number => typeof value === "number");
	const validCost = (value: string | null | undefined) =>
		value?.trim() !== "" && value != null && Number.isFinite(Number(value));
	const costObservations = steps.flatMap((step) => {
		const usage = step.usage;
		const value = usage?.estimatedCost;
		if (!usage || typeof value !== "string" || !validCost(value)) return [];
		return [
			{
				value: value.trim(),
				currency: usage.currency,
				priceUnit: usage.priceUnit,
				coverage: usage.coverage,
				inputPrice: usage.inputPrice,
				outputPrice: usage.outputPrice,
				cacheReadPrice: usage.cacheReadPrice,
				cacheWritePrice: usage.cacheWritePrice,
				priceSource: usage.priceSource,
				priceCheckedAt: usage.priceCheckedAt,
			},
		];
	});
	const input = metric("inputTokens");
	const output = metric("outputTokens");
	const cacheRead = metric("cacheReadTokens");
	const cacheWrite = metric("cacheWriteTokens");
	const comparable =
		costObservations.length === steps.length &&
		steps.length > 0 &&
		costObservations.every(
			(item) =>
				item.coverage === "complete" &&
				item.currency &&
				item.priceUnit === costObservations[0]?.priceUnit &&
				item.currency === costObservations[0]?.currency,
		);
	const exactCost = (values: string[]) => {
		const parsed = values.map((value) => /^(\d+)(?:\.(\d+))?$/.exec(value));
		if (parsed.some((value) => value === null)) return null;
		const components = parsed.filter(
			(value): value is RegExpExecArray => value !== null,
		);
		const scale = Math.max(...components.map((value) => value[2]?.length ?? 0));
		const total = components.reduce(
			(sum, value) =>
				sum + BigInt(value[1] + (value[2] ?? "").padEnd(scale, "0")),
			0n,
		);
		const digits = total.toString().padStart(scale + 1, "0");
		return scale
			? `${digits.slice(0, -scale)}.${digits.slice(-scale)}`.replace(
					/\.?0+$/,
					"",
				) || "0"
			: digits;
	};
	const costTotal = comparable
		? exactCost(costObservations.map((item) => item.value))
		: null;
	const serverHasSummary =
		serverSummary !== undefined && serverSummary !== null;
	const cost = serverHasSummary
		? serverSummary.coverage === "complete" && serverSummary.cost
			? serverSummary.cost
			: "unavailable"
		: (costTotal ?? "unavailable");
	return {
		attempts: steps.length,
		retries: steps.reduce(
			(count, step, index) =>
				count +
				(steps.slice(0, index).some((prior) => prior.stepId === step.stepId)
					? 1
					: 0),
			0,
		),
		durationMs: durations.length
			? durations.reduce((sum, value) => sum + value, 0)
			: null,
		durationCoverage: coverage(durations.length),
		inputTokens: input.value,
		inputTokenCoverage: input.coverage,
		outputTokens: output.value,
		outputTokenCoverage: output.coverage,
		cacheReadTokens: cacheRead.value,
		cacheReadTokenCoverage: cacheRead.coverage,
		cacheWriteTokens: cacheWrite.value,
		cacheWriteTokenCoverage: cacheWrite.coverage,
		cost,
		costCurrency: serverHasSummary
			? serverSummary.coverage === "complete"
				? (serverSummary.currency ?? null)
				: null
			: comparable
				? costObservations[0]?.currency
				: null,
		costCoverage: serverHasSummary
			? serverSummary.coverage === "complete"
				? "complete"
				: "unavailable"
			: coverage(costObservations.length),
		costObservations,
		priceSources: [
			...new Set(
				costObservations
					.map((usage) => usage.priceSource)
					.filter((value): value is string => Boolean(value)),
			),
		],
		priceCheckedAt: [
			...new Set(
				costObservations
					.map((usage) => usage.priceCheckedAt)
					.filter((value): value is string => Boolean(value)),
			),
		],
	};
}
