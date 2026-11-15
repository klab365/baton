import { expect, test } from "@playwright/test";

test("supported Rig level saves, reloads and can be cleared to runtime default", async ({
	page,
}) => {
	let savedAgent: Record<string, unknown> | undefined;
	await page.route("**/api/**", async (route) => {
		const request = route.request();
		const path = new URL(request.url()).pathname;
		if (path === "/api/project")
			return route.fulfill({ json: { status: "ready", name: "Test" } });
		if (path === "/api/version")
			return route.fulfill({ json: { version: "test" } });
		if (path === "/api/credentials/copilot/models")
			return route.fulfill({
				json: [{ id: "gpt-5.3-codex", label: "Copilot GPT-5.3 Codex" }],
			});
		if (
			path.startsWith("/api/agents") &&
			["POST", "PUT"].includes(request.method())
		) {
			savedAgent = request.postDataJSON().agent;
			return route.fulfill({
				json: { revision: "next", agents: [savedAgent] },
			});
		}
		if (path.startsWith("/api/agents"))
			return route.fulfill({
				json: {
					revision: "current",
					agents: savedAgent ? [savedAgent] : [],
					skills: [],
				},
			});
		if (path === "/api/credentials")
			return route.fulfill({
				json: [
					{
						id: "copilot",
						name: "Copilot",
						credentialType: "github_copilot_session",
						status: "available",
					},
				],
			});
		if (path === "/api/credential-types")
			return route.fulfill({
				json: [
					{
						id: "github_copilot_session",
						label: "GitHub Copilot",
						provider: "copilot",
					},
				],
			});
		return route.fulfill({ json: {} });
	});

	await page.goto("/agents");
	await page.getByRole("button", { name: /Create agent/ }).click();
	await page.getByLabel("Agent ID").fill("writer");
	await page.getByLabel("Instructions").fill("Write clearly");
	await page.getByLabel("Credential").click();
	await page.getByRole("option", { name: "Copilot" }).click();
	await expect(
		page.getByRole("combobox", { name: "Model (required)" }),
	).toBeEnabled();
	await page.getByRole("combobox", { name: "Model (required)" }).click();
	await page.getByRole("option", { name: "Copilot GPT-5.3 Codex" }).click();
	await page.getByLabel("Thinking level").click();
	await page.getByRole("option", { name: "high", exact: true }).click();
	await page.getByRole("button", { name: "Save agent" }).click();
	await expect.poll(() => savedAgent?.thinkingLevel).toBe("high");
	await expect(savedAgent?.model).toBe("copilot/gpt-5.3-codex");

	await page.getByRole("button", { name: /writer/ }).click();
	await expect(page.getByLabel("Thinking level")).toContainText("high");
	await page.getByLabel("Thinking level").click();
	await page.getByRole("option", { name: "Runtime default" }).click();
	await page.getByRole("button", { name: "Save agent" }).click();
	await expect.poll(() => savedAgent?.thinkingLevel).toBeUndefined();
});
