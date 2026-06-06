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
  let transitionTimeout;

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
    document.documentElement.classList.add("theme-transition");
    document.documentElement.dataset.theme = nextTheme;
    localStorage.setItem("theme", nextTheme);
    updateToggle();
    clearTimeout(transitionTimeout);
    transitionTimeout = setTimeout(() => {
      document.documentElement.classList.remove("theme-transition");
    }, 500);
  });

  updateToggle();
});

// static/theme.js (Updated engine block)

let hasRunViewTransitionsEngine = false;

const runViewTransitionsEngine = () => {
  if (hasRunViewTransitionsEngine) return;

  // 1. Map images
  document.querySelectorAll("[data-vt-img]").forEach((el) => {
    const slug = el.getAttribute("data-vt-img");
    if (slug) {
      el.style.viewTransitionName = `img-${slug}`;
      el.style.viewTransitionClass = "fluid-image";
    }
  });

  // 2. Map titles
  document.querySelectorAll("[data-vt-title]").forEach((el) => {
    const slug = el.getAttribute("data-vt-title");
    if (slug) {
      el.style.viewTransitionName = `title-${slug}`;
      el.style.viewTransitionClass = "fluid-text";
    }
  });

  hasRunViewTransitionsEngine = true;
};

// 3. FIXED FOR IN-PAGE LINKS: Intercept clicks on links going back to projects
document.addEventListener("click", (e) => {
  const link = e.target.closest("a");
  if (!link) return;

  const href = link.getAttribute("href");
  
  // If clicking an in-page link to the list page or a specific project page
  if (href === "/projects" || href?.startsWith("/projects") || href?.endsWith("/projects")) {
    // Force the current page's transition names to stay locked in right before the native navigation fires
    runViewTransitionsEngine();
  }
});

// Run immediately on parse
if (document.readyState === "loading") {
  document.addEventListener("DOMContentLoaded", runViewTransitionsEngine);
} else {
  runViewTransitionsEngine();
}

// Re-fire for browser cache back/forward navigations
window.addEventListener("pageshow", () => {
  runViewTransitionsEngine();
});