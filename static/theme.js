(() => {
  const storedTheme = localStorage.getItem("theme");
  const systemTheme = window.matchMedia("(prefers-color-scheme: dark)").matches
    ? "dark"
    : "light";

  document.documentElement.dataset.theme = storedTheme || systemTheme;
})();

document.addEventListener("DOMContentLoaded", () => {
  const toggle = document.querySelector(".theme-toggle");
  if (!toggle) return;

  const updateToggle = () => {
    const theme = document.documentElement.dataset.theme;
    const nextTheme = theme === "dark" ? "light" : "dark";
    toggle.setAttribute("aria-label", `Switch to ${nextTheme} mode`);
    toggle.setAttribute("title", `Switch to ${nextTheme} mode`);
    toggle.setAttribute("aria-pressed", String(theme === "dark"));
  };

  toggle.addEventListener("click", () => {
    const theme = document.documentElement.dataset.theme;
    const nextTheme = theme === "dark" ? "light" : "dark";
    document.documentElement.dataset.theme = nextTheme;
    localStorage.setItem("theme", nextTheme);
    updateToggle();
  });

  updateToggle();
});
