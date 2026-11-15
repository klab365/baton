import { expect, test } from "@playwright/test";

test("pipeline editor defaults global attempts to 50 and saves an edited limit", async ({
	page,
}) => {
	let saved: { pipeline: { maxAttempts: number } } | undefined;
	await page.route("**/api/**", async (route) => {
		const path = new URL(route.request().url()).pathname;
		if (path === "/api/project") {
			await route.fulfill({
				json: {
					status: "ready",
					name: "Test",
					config: { pipelines: [{ id: "factory" }] },
				},
			});
		} else if (
			path === "/api/pipelines" &&
			route.request().method() === "GET"
		) {
			await route.fulfill({
				json: {
					revision: "rev",
					agents: [{ id: "builder" }],
					pipelines: [
						{
							id: "factory",
							maxAttempts: saved?.pipeline.maxAttempts,
							steps: [
								{
									id: "build",
									name: "Build",
									type: "agent",
									agentId: "builder",
									prompt: "",
									transitions: [],
								},
							],
						},
					],
				},
			});
		} else if (
			path === "/api/pipelines/factory" &&
			route.request().method() === "PUT"
		) {
			saved = route.request().postDataJSON();
			await route.fulfill({ json: {} });
		} else await route.fulfill({ json: {} });
	});
	await page.goto("/pipelines");
	await page.getByRole("button", { name: /factory/ }).click();
	const limit = page.getByLabel("Maximum pipeline attempts");
	await expect(limit).toHaveValue("50");
	await limit.fill("7");
	await page.getByRole("button", { name: "Build", exact: true }).click();
	await page.getByRole("button", { name: "Save changes" }).click();
	await expect(page.getByRole("status")).toContainText("Pipeline saved");
	expect(saved).toMatchObject({ pipeline: { maxAttempts: 7 } });
	await page.getByRole("button", { name: /factory/ }).click();
	await expect(limit).toHaveValue("7");
});

test("pipeline attempt limit rejects empty, zero, out-of-range and fractional values", async ({
	page,
}) => {
	let writes = 0;
	await page.route("**/api/**", async (route) => {
		const path = new URL(route.request().url()).pathname;
		if (path === "/api/project")
			return route.fulfill({
				json: {
					status: "ready",
					name: "Test",
					config: { pipelines: [{ id: "factory" }] },
				},
			});
		if (path === "/api/pipelines" && route.request().method() === "GET")
			return route.fulfill({
				json: {
					revision: "rev",
					agents: [{ id: "builder" }],
					pipelines: [
						{
							id: "factory",
							maxAttempts: 50,
							steps: [
								{
									id: "build",
									name: "Build",
									type: "agent",
									agentId: "builder",
									prompt: "",
									transitions: [],
								},
							],
						},
					],
				},
			});
		if (["POST", "PUT"].includes(route.request().method())) writes++;
		return route.fulfill({ json: {} });
	});
	await page.goto("/pipelines");
	await page.getByRole("button", { name: /factory/ }).click();
	const limit = page.getByLabel("Maximum pipeline attempts");
	for (const value of ["", "0", "1001", "1.5"]) {
		await limit.fill(value);
		await page.getByRole("button", { name: "Save changes" }).click();
		expect(
			await limit.evaluate((input: HTMLInputElement) => input.validity.valid),
		).toBe(false);
	}
	expect(writes).toBe(0);
});

test("changing step type preserves its per-step attempt limit", async ({
	page,
}) => {
	let saved: unknown;
	await page.route("**/api/**", async (route) => {
		const path = new URL(route.request().url()).pathname;
		if (path === "/api/project") {
			await route.fulfill({
				json: {
					status: "ready",
					name: "Test",
					config: { pipelines: [{ id: "factory" }] },
				},
			});
		} else if (
			path === "/api/pipelines" &&
			route.request().method() === "GET"
		) {
			await route.fulfill({
				json: {
					revision: "rev",
					agents: [{ id: "builder" }],
					pipelines: [
						{
							id: "factory",
							steps: [
								{
									id: "build",
									name: "Build",
									type: "agent",
									agentId: "builder",
									prompt: "",
									maxAttempts: 3,
									transitions: [],
								},
							],
						},
					],
				},
			});
		} else if (
			path === "/api/pipelines/factory" &&
			route.request().method() === "PUT"
		) {
			saved = route.request().postDataJSON();
			await route.fulfill({ json: {} });
		} else await route.fulfill({ json: {} });
	});
	await page.goto("/pipelines");
	await page.getByRole("button", { name: /factory/ }).click();
	await page.getByRole("button", { name: "Build", exact: true }).click();
	await page.getByLabel("Step type").click();
	await page.getByRole("option", { name: "Human wait" }).click();
	await page.getByRole("button", { name: "Save changes" }).click();
	await expect(page.getByRole("status")).toContainText("Pipeline saved");
	expect(saved).toMatchObject({
		pipeline: { steps: [{ id: "build", type: "human", maxAttempts: 3 }] },
	});
});

test("pipeline editor defaults to omitting the task and saves the selected setting", async ({
	page,
}) => {
	let saved: unknown;
	await page.route("**/api/**", async (route) => {
		const path = new URL(route.request().url()).pathname;
		if (path === "/api/project") {
			await route.fulfill({
				json: {
					status: "ready",
					name: "Test",
					config: { pipelines: [{ id: "factory" }] },
				},
			});
		} else if (
			path === "/api/pipelines" &&
			route.request().method() === "GET"
		) {
			await route.fulfill({
				json: {
					revision: "rev",
					agents: [{ id: "builder" }],
					pipelines: [
						{
							id: "factory",
							steps: [
								{
									id: "build",
									name: "Build",
									type: "agent",
									agentId: "builder",
									prompt: "",
									transitions: [],
								},
							],
						},
					],
				},
			});
		} else if (
			path === "/api/pipelines/factory" &&
			route.request().method() === "PUT"
		) {
			saved = route.request().postDataJSON();
			await route.fulfill({ json: {} });
		} else {
			await route.fulfill({ json: {} });
		}
	});
	await page.goto("/pipelines");
	await page.getByRole("button", { name: /factory/ }).click();
	await page.getByRole("button", { name: "Build", exact: true }).click();
	const picker = page.getByRole("combobox", { name: "Original task" });
	await expect(picker).toContainText("Omit original task");
	await picker.focus();
	await page.keyboard.press("Enter");
	await page.getByRole("option", { name: "Include original task" }).click();
	await page.getByRole("button", { name: "Save changes" }).click();
	await expect(page.getByRole("status")).toContainText("Pipeline saved");
	expect(saved).toMatchObject({
		pipeline: { steps: [{ id: "build", includeTask: true }] },
	});
});
