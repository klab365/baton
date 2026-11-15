import { expect, test } from "@playwright/test";

test("failure summary links to the matching saved timeline entry", async ({
	page,
}) => {
	const run = {
		id: "failed-run",
		pipelineId: "factory",
		task: "Example",
		startedAt: "2026-01-01T00:00:00Z",
		status: "failed",
		events: [],
		trace: [],
		pendingActions: [],
		configSnapshot: null,
		steps: [
			{ stepId: "legacy", attempt: 0, status: "succeeded" },
			{
				stepId: "review",
				attempt: 7,
				status: "failed",
				error: "review failed",
			},
		],
	};
	await page.route("**/api/**", (route) => {
		const path = new URL(route.request().url()).pathname;
		if (path === "/api/runs/failed-run/events") return route.abort();
		if (path === "/api/runs/failed-run") return route.fulfill({ json: run });
		return route.fulfill({
			json: { status: "ready", name: "Test", config: { pipelines: [] } },
		});
	});
	await page.goto("/runs/failed-run");
	await expect(
		page.getByRole("link", { name: "View attempt 7 in timeline" }),
	).toHaveAttribute("href", "#attempt-entry-1");
});

test("compares legacy visits and attributes structured evidence to its agent", async ({
	page,
}) => {
	const run = {
		id: "run-handoffs",
		pipelineId: "factory",
		task: "Example",
		startedAt: "2026-01-01T00:00:00Z",
		status: "succeeded",
		events: [],
		trace: [],
		pendingActions: [],
		configSnapshot: null,
		steps: [
			{
				stepId: "review",
				agentId: "reviewer",
				attempt: 0,
				status: "succeeded",
				input: null,
				output: "No finding reported yet",
			},
			{
				stepId: "review",
				agentId: "reviewer",
				attempt: 0,
				status: "succeeded",
				input: "first received",
				output: "first output",
				structuredHandoff: {
					version: 1,
					summary: "First finding report",
					findings: [
						{ id: "F-1", status: "open", description: "Missing test" },
					],
					verification: [
						{ description: "bun test", result: "passed" },
						{ description: "bun test", result: "passed" },
					],
				},
			},
			{
				stepId: "review",
				agentId: "reviewer",
				attempt: 0,
				status: "succeeded",
				input: null,
				output: null,
				structuredHandoff: {
					version: 1,
					summary: "Finding addressed",
					findings: [
						{ id: "F-1", status: "resolved", description: "Added test" },
					],
					verification: [],
				},
			},
		],
	};
	await page.route("**/api/**", (route) => {
		const path = new URL(route.request().url()).pathname;
		if (path === "/api/runs/run-handoffs/events") return route.abort();
		if (path === "/api/runs/run-handoffs") return route.fulfill({ json: run });
		return route.fulfill({
			json: { status: "ready", name: "Test", config: { pipelines: [] } },
		});
	});
	await page.goto("/runs/run-handoffs");
	await expect(page.getByText("No finding reported yet")).toBeVisible({
		timeout: 15000,
	});
	const visits = page.locator(".attempt-comparison a");
	await expect(visits).toHaveCount(9);
	await expect(
		visits.filter({ hasText: "Saved visit 1" }).first(),
	).toBeVisible();
	await expect(
		visits.filter({ hasText: "Saved visit 3" }).first(),
	).toBeVisible();
	const secondVisit = visits.filter({ hasText: "Saved visit 3" }).first();
	await secondVisit.focus();
	await page.keyboard.press("Enter");
	await expect(page).toHaveURL(/#attempt-entry-2/);
	await page
		.getByText("Structured handoff (reported by reviewer)")
		.last()
		.click();
	await expect(page.getByText("F-1", { exact: false }).last()).toBeVisible();
	await expect(page.getByText("Visit 2: open").last()).toBeVisible();
	await expect(page.getByText("Visit 3: resolved").last()).toBeVisible();
	await expect(
		page.getByText(
			"bun test: passed — reported by reviewer; not independently verified.",
		),
	).toHaveCount(2);
	await expect(
		page.getByText("Received handoff unavailable.").first(),
	).toBeVisible();
	await expect(
		page.getByText("Produced handoff unavailable.").first(),
	).toBeVisible();
});
