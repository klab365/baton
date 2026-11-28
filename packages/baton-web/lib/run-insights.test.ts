import { expect, test } from "bun:test";
import { runInsights } from "./run-insights";

test("keeps run cost unavailable while retaining individual zero cost and provenance", () => {
	const result = runInsights([
		{
			stepId: "build",
			attempt: 1,
			status: "succeeded",
			durationMs: 0,
			usage: {
				inputTokens: 0,
				outputTokens: 5,
				cacheReadTokens: 2,
				estimatedCost: "0",
				priceSource: "catalog",
				priceCheckedAt: "now",
			},
		},
		{
			stepId: "review",
			attempt: 2,
			status: "succeeded",
			durationMs: null,
			usage: {
				inputTokens: 3,
				outputTokens: null,
				cacheReadTokens: null,
				estimatedCost: "0.02",
				priceSource: "other",
			},
		},
	]);
	expect(result.cost).toBe("unavailable");
	expect(result.costCoverage).toBe("complete");
	expect(result.costObservations.map((cost) => cost.value)).toEqual([
		"0",
		"0.02",
	]);
	expect(result.priceSources).toEqual(["catalog", "other"]);
	expect(result.durationMs).toBe(0);
	expect(result.durationCoverage).toBe("partial");
	expect(result.inputTokenCoverage).toBe("complete");
	expect(result.outputTokenCoverage).toBe("partial");
	expect(result.cacheReadTokens).toBe(2);
});

test("uses the server run summary when supplied and keeps unavailable totals gated", () => {
	const steps = [
		{
			stepId: "build",
			attempt: 1,
			status: "succeeded",
			usage: {
				estimatedCost: "0.02",
				currency: "USD",
				priceUnit: "perMillionTokens",
				coverage: "complete",
			},
		},
	];
	expect(
		runInsights(steps, { cost: "0.02", currency: "USD", coverage: "complete" })
			.cost,
	).toBe("0.02");
	expect(
		runInsights(steps, { cost: null, currency: null, coverage: "unavailable" })
			.cost,
	).toBe("unavailable");
});

test("does not mistake multiple ordinary steps' global numbers for retries; filters pending", () => {
	const result = runInsights([
		{ stepId: "build", attempt: 1, status: "succeeded" },
		{ stepId: "review", attempt: 2, status: "running" },
		{ stepId: "finish", attempt: 0, status: "pending" },
	]);
	expect(result.attempts).toBe(2);
	expect(result.retries).toBe(0);
});

test("counts a repeated step visit as a retry", () => {
	const result = runInsights([
		{ stepId: "build", attempt: 1, status: "succeeded" },
		{ stepId: "review", attempt: 2, status: "succeeded" },
		{ stepId: "build", attempt: 3, status: "running" },
	]);
	expect(result.retries).toBe(1);
});

test("retains category rate provenance in cost observations", () => {
	const result = runInsights([
		{
			stepId: "a",
			attempt: 1,
			status: "succeeded",
			usage: {
				estimatedCost: "0.1",
				currency: "USD",
				priceUnit: "perMillionTokens",
				coverage: "complete",
				inputPrice: "2",
				outputPrice: "8",
				cacheReadPrice: "0.2",
				cacheWritePrice: "2",
			},
		},
	]);
	expect(result.costObservations[0]).toMatchObject({
		inputPrice: "2",
		outputPrice: "8",
		cacheReadPrice: "0.2",
		cacheWritePrice: "2",
	});
});

test("sums only complete costs with matching currency and price unit", () => {
	const result = runInsights([
		{
			stepId: "a",
			attempt: 1,
			status: "succeeded",
			usage: {
				estimatedCost: "0.10",
				currency: "USD",
				priceUnit: "perMillionTokens",
				coverage: "complete",
			},
		},
		{
			stepId: "b",
			attempt: 2,
			status: "succeeded",
			usage: {
				estimatedCost: "0.25",
				currency: "USD",
				priceUnit: "perMillionTokens",
				coverage: "complete",
			},
		},
	]);
	expect(result.cost).toBe("0.35");
	expect(result.costCurrency).toBe("USD");
});

test("does not total estimates across currencies or incomplete coverage", () => {
	const costs = (currency: string, coverage = "complete") => ({
		estimatedCost: "0.1",
		currency,
		priceUnit: "tokens",
		coverage,
	});
	const mixed = runInsights([
		{ stepId: "a", attempt: 1, status: "succeeded", usage: costs("USD") },
		{ stepId: "b", attempt: 2, status: "succeeded", usage: costs("EUR") },
	]);
	const mixedUnits = runInsights([
		{ stepId: "a", attempt: 1, status: "succeeded", usage: costs("USD") },
		{
			stepId: "b",
			attempt: 2,
			status: "succeeded",
			usage: { ...costs("USD"), priceUnit: "perToken" },
		},
	]);
	const partial = runInsights([
		{ stepId: "a", attempt: 1, status: "succeeded", usage: costs("USD") },
		{
			stepId: "b",
			attempt: 2,
			status: "succeeded",
			usage: costs("USD", "partial"),
		},
	]);
	expect(mixed.cost).toBe("unavailable");
	expect(mixedUnits.cost).toBe("unavailable");
	expect(mixedUnits.costCurrency).toBeNull();
	expect(partial.cost).toBe("unavailable");
});

test("adds cost decimals without binary floating-point rounding", () => {
	const result = runInsights([
		{
			stepId: "a",
			attempt: 1,
			status: "succeeded",
			usage: {
				estimatedCost: "0.1",
				currency: "USD",
				priceUnit: "tokens",
				coverage: "complete",
			},
		},
		{
			stepId: "b",
			attempt: 2,
			status: "succeeded",
			usage: {
				estimatedCost: "0.2",
				currency: "USD",
				priceUnit: "tokens",
				coverage: "complete",
			},
		},
	]);
	expect(result.cost).toBe("0.3");
});

test("preserves large decimal totals exactly and normalizes trailing zeroes", () => {
	const result = runInsights([
		{
			stepId: "a",
			attempt: 1,
			status: "succeeded",
			usage: {
				estimatedCost: "9007199254740993.10",
				currency: "USD",
				priceUnit: "tokens",
				coverage: "complete",
			},
		},
		{
			stepId: "b",
			attempt: 2,
			status: "succeeded",
			usage: {
				estimatedCost: "0.20",
				currency: "USD",
				priceUnit: "tokens",
				coverage: "complete",
			},
		},
	]);
	expect(result.cost).toBe("9007199254740993.3");
});

test("does not total scientific-notation estimates without exact decimal parsing", () => {
	const result = runInsights([
		{
			stepId: "a",
			attempt: 1,
			status: "succeeded",
			usage: {
				estimatedCost: "1e-7",
				currency: "USD",
				priceUnit: "tokens",
				coverage: "complete",
			},
		},
	]);
	expect(result.cost).toBe("unavailable");
});

test("keeps missing, empty and invalid cost strings unavailable", () => {
	const result = runInsights([
		{
			stepId: "a",
			attempt: 1,
			status: "succeeded",
			usage: { estimatedCost: "" },
		},
		{
			stepId: "b",
			attempt: 2,
			status: "succeeded",
			usage: { estimatedCost: "NaN" },
		},
	]);
	expect(result.cost).toBe("unavailable");
	expect(result.costObservations).toEqual([]);
});
