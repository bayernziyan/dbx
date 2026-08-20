(() => {
  "use strict";

  const roleCookie = "dbx_guard_ui";
  const hiddenAttribute = "data-dbx-guard-hidden";
  const forbiddenIconClasses = [
    "lucide-settings",
    "lucide-cloud-download",
    "lucide-sun",
    "lucide-moon",
    "lucide-monitor",
    "lucide-palette",
  ];

  function cookieValue(name) {
    const prefix = `${name}=`;
    const item = document.cookie.split(";").map((value) => value.trim()).find((value) => value.startsWith(prefix));
    return item ? decodeURIComponent(item.slice(prefix.length)) : "";
  }

  function viewer() {
    return cookieValue(roleCookie) === "viewer";
  }

  function isForbiddenControl(element) {
    const text = `${element.getAttribute("aria-label") || ""} ${element.getAttribute("title") || ""} ${element.textContent || ""}`;
    if (/GitHub|检查更新|Check for updates|设置|Settings|主题|Theme/i.test(text)) return true;
    if (forbiddenIconClasses.some((name) => element.querySelector(`.${name}`))) return true;
    return element.innerHTML.includes("M12 0C5.37 0 0 5.37");
  }

  function applyRole() {
    const isViewer = viewer();
    document.documentElement.classList.toggle("dbx-guard-viewer", isViewer);
    if (!isViewer) {
      document.querySelectorAll(`[${hiddenAttribute}]`).forEach((element) => {
        element.removeAttribute(hiddenAttribute);
      });
      return;
    }
    document.querySelectorAll("button,a,[role='menuitem']").forEach((element) => {
      if (isForbiddenControl(element)) element.setAttribute(hiddenAttribute, "true");
    });
  }

  document.addEventListener("keydown", (event) => {
    if (!viewer()) return;
    if ((event.ctrlKey || event.metaKey) && (event.key === "," || event.code === "Comma")) {
      event.preventDefault();
      event.stopImmediatePropagation();
    }
  }, true);

  new MutationObserver(applyRole).observe(document.documentElement, { childList: true, subtree: true, attributes: false });
  window.setInterval(applyRole, 750);
  applyRole();
})();
