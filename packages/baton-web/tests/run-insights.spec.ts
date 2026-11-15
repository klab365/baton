import { expect, test } from "@playwright/test";

for (const status of ["running", "succeeded"]) {
	test(`shows persisted run insights for ${status} runs`, async ({ page }) => {
		const run = {
			id: "run-insights",
			pipelineId: "factory",
			task: "Example",
			startedAt: "2026-01-01T00:00:00Z",
			endedAt: status === "running" ? null : "2026-01-01T00:00:01Z",
			durationMs: 1000,
			status,
			currentStep: status === "running" ? "build" : null,
			taskId: null,
			events: [],
			trace: [],
			pendingActions: [],
			configSnapshot: null,
			steps: [
				{
					stepId: "build",
					agentId: "builder",
					attempt: 1,
					status: "succeeded",
					durationMs: 0,
					input: null,
					output: "saved result",
					error: null,
					usage: {
						inputTokens: 0,
						outputTokens: 3,
						cacheReadTokens: null,
						cacheWriteTokens: null,
						estimatedCost: "0",
						priceSource: "test",
						priceCheckedAt: "2026-01-01T00:00:00Z",
					},
				},
				{
					stepId: "review",
					agentId: "reviewer",
					attempt: 2,
					status: status === "running" ? "running" : "succeeded",
					durationMs: null,
					input: null,
					output: null,
					error: null,
					usage: null,
				},
			],
		};
		await page.route("**/api/**", (route) => {
			const path = new URL(route.request().url()).pathname;
			if (path === "/api/runs/run-insights/events") return route.abort();
			if (path === "/api/runs/run-insights")
				return route.fulfill({ json: run });
			return route.fulfill({
				json: { status: "ready", name: "Test", config: { pipelines: [] } },
			});
		});
		await page.goto("/runs/run-insights");
		await expect(
			page.getByText(/recorded step duration: 0 ms \(partial\)/),
		).toBeVisible({ timeout: 15000 });
		await expect(page.getByText(/Input tokens: 0 \(partial\)/)).toBeVisible({
			timeout: 15000,
		});
		await expect(page.getByText(/output tokens: 3 \(partial\)/)).toBeVisible();
		await expect(page.getByText(/Run cost: not available/)).toBeVisible();
		await expect(page.getByText("saved result")).toBeVisible();
	});
}

test("shows the persisted pipeline exhaustion reason", async ({ page }) => {
	const reason = "Pipeline exhausted maxAttempts limit of 50";
	await page.route("**/api/**", (route) => {
		const path = new URL(route.request().url()).pathname;
		if (path === "/api/runs/run-limit/events") return route.abort();
		if (path === "/api/runs/run-limit")
			return route.fulfill({
				json: {
					id: "run-limit",
					pipelineId: "factory",
					task: "Example",
					startedAt: "2026-01-01T00:00:00Z",
					status: "limitExhausted",
					currentStep: null,
					events: [
						{
							runId: "run-limit",
							status: "limitExhausted",
							occurredAt: "2026-01-01T00:00:01Z",
							message: reason,
						},
					],
					steps: [],
					trace: [],
					pendingActions: [],
					configSnapshot: null,
				},
			});
		return route.fulfill({
			json: { status: "ready", name: "Test", config: { pipelines: [] } },
		});
	});
	await page.goto("/runs/run-limit");
	await expect(
		page
			.getByRole("region", { name: "Run overview" })
			.getByText(reason, { exact: true }),
	).toBeVisible({ timeout: 15000 });
});
