// Navigation shared by UI checks after the workbench replaced the fixed tool rail.
export async function openEditor(page, label) {
  const tab = page
    .locator(".editor-tabs")
    .getByRole("button", { name: label, exact: true });
  if (await tab.count()) return tab.click();
  await page.getByRole("menuitem", { name: "工具", exact: true }).click();
  await page.getByRole("menuitem", { name: label, exact: true }).click();
}
export async function openWindowPanel(page, label) {
  await page.getByRole("menuitem", { name: "窗口", exact: true }).click();
  await page.getByRole("menuitem", { name: label, exact: true }).click();
}
