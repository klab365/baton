import { expect, test } from "@playwright/test";

test.beforeEach(async ({ page }) => {
	await page.route("**/api/**", async (route) => {
		const path = new URL(route.request().url()).pathname;
		if (path === "/api/project") {
			await route.fulfill({
				json: {
					status: "ready",
					name: "Test",
					config: { pipelines: [{ id: "default" }] },
				},
			});
		} else if (path === "/api/skills") {
			await route.fulfill({ json: { skills: [] } });
		} else {
			await route.fulfill({ json: {} });
		}
	});
});

test("task description previews Markdown and saves exact source", async ({
	page,
}) => {
	await page.goto("/tasks/new");
	await page.getByLabel("Title").fill("Example");
	const input = page.locator("#description");
	const source =
		"# Heading\n\n**bold** and *italic*\n\n- first\n- second\n\n[link](https://example.com)\n";
	await input.fill(source);
	const height = await input.evaluate(
		(element) => element.getBoundingClientRect().height,
	);
	await input.fill(`${source}\n${"Long line of text\n".repeat(30)}`);
	await expect
		.poll(() =>
			input.evaluate((element) => element.getBoundingClientRect().height),
		)
		.toBeGreaterThan(height);
	await input.fill(source);
	const previewButton = page.getByRole("button", { name: "Markdown Preview" });
	await previewButton.focus();
	await page.keyboard.press("Enter");
	await expect(previewButton).toHaveAttribute("aria-pressed", "true");
	const preview = page.getByRole("region", { name: "Markdown preview" });
	await expect(preview.getByRole("heading", { name: "Heading" })).toBeVisible();
	await expect(preview.locator("strong")).toHaveText("bold");
	await expect(preview.locator("em")).toHaveText("italic");
	await expect(preview.locator("li")).toHaveCount(2);
	await expect(preview.getByRole("link", { name: "link" })).toHaveAttribute(
		"href",
		"https://example.com",
	);
	await page.getByRole("button", { name: "Text", exact: true }).click();
	await expect(input).toHaveValue(source);
	await page.getByRole("combobox", { name: "Pipeline" }).click();
	await page.getByRole("option", { name: "default" }).click();
	const request = page.waitForRequest(
		(request) =>
			request.url().endsWith("/api/tasks") && request.method() === "POST",
	);
	await page.getByRole("button", { name: "Create task" }).click();
	expect((await request).postDataJSON().description).toBe(source);
});

test("submitting an empty required field from preview reveals and focuses it", async ({
	page,
}) => {
	await page.goto("/skills");
	await page.getByRole("button", { name: "Create skill" }).click();
	await page.getByLabel("Skill ID").fill("example");
	await page.locator("#skill-instructions").fill("Instructions");
	await page.getByRole("button", { name: "Markdown Preview" }).first().click();
	await page.getByRole("button", { name: "Save skill" }).click();
	await expect(page.locator("#skill-description")).toBeVisible();
	await expect(page.locator("#skill-description")).toBeFocused();
});
