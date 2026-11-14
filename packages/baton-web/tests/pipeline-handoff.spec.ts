import { expect, test } from "@playwright/test";

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
