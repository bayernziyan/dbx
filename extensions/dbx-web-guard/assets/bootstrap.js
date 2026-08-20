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
  let firstTemplateName = "";
  let templatesLoaded = false;
  let templateSelectionBusy = false;
  let agentSelectionBusy = false;

  const nativeFetch = window.fetch.bind(window);
  window.fetch = async (...args) => {
    const response = await nativeFetch(...args);
    try {
      const request = args[0];
      const url = new URL(typeof request === "string" ? request : request.url, window.location.href);
      const method = (args[1]?.method || (typeof request === "object" && request.method) || "GET").toUpperCase();
      if (viewer() && method === "GET" && url.pathname.endsWith("/api/prompt-templates")) {
        const templates = await response.clone().json();
        templatesLoaded = Array.isArray(templates);
        firstTemplateName = templates[0]?.name || "";
      }
    } catch {}
    return response;
  };

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

  function hasChevron(button) {
    return !!button.querySelector('svg path[d="m6 9 6 6 6-6"]');
  }

  function templateTrigger() {
    return [...document.querySelectorAll("button")].find(
      (button) => button.querySelector(".lucide-file-code") && hasChevron(button) && button.className.includes("max-w-[40%]"),
    );
  }

  function enforceViewerTemplate() {
    const trigger = templateTrigger();
    if (!trigger) return;
    if (templatesLoaded && !firstTemplateName) {
      trigger.disabled = true;
      trigger.setAttribute("aria-disabled", "true");
      return;
    }
    if (firstTemplateName && trigger.textContent.includes(firstTemplateName)) {
      trigger.disabled = true;
      trigger.setAttribute("aria-disabled", "true");
      templateSelectionBusy = false;
      return;
    }
    trigger.disabled = false;
    trigger.removeAttribute("aria-disabled");
    if (!firstTemplateName || templateSelectionBusy) return;
    templateSelectionBusy = true;
    trigger.click();
    window.setTimeout(() => {
      const option = [...document.querySelectorAll("button")].find(
        (button) => button.querySelector(".font-medium")?.textContent?.trim() === firstTemplateName,
      );
      if (option) option.click();
      window.setTimeout(() => {
        templateSelectionBusy = false;
        applyRole();
      }, 80);
    }, 80);
  }

  function modeTrigger() {
    return [...document.querySelectorAll("button")].find(
      (button) => hasChevron(button) && (button.querySelector(".lucide-message-square-plus") || button.querySelector(".lucide-bot")),
    );
  }

  function enforceViewerAgentMode() {
    document.querySelectorAll("button .lucide-message-square-plus").forEach((icon) => {
      const button = icon.closest("button");
      if (button && !hasChevron(button)) button.setAttribute(hiddenAttribute, "true");
    });
    const trigger = modeTrigger();
    if (!trigger || trigger.querySelector(".lucide-bot") || agentSelectionBusy) return;
    agentSelectionBusy = true;
    trigger.click();
    window.setTimeout(() => {
      const agentButton = [...document.querySelectorAll("button")].find(
        (button) => button.querySelector(".lucide-bot") && !hasChevron(button) && /Agent|代理|智能体/i.test(button.textContent || ""),
      );
      if (agentButton) agentButton.click();
      agentSelectionBusy = false;
    }, 80);
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
    enforceViewerTemplate();
    enforceViewerAgentMode();
  }

  document.addEventListener("click", (event) => {
    if (!viewer()) return;
    const button = event.target.closest?.("button");
    if (button?.querySelector(".lucide-message-square-plus") && !hasChevron(button)) {
      event.preventDefault();
      event.stopImmediatePropagation();
    }
  }, true);

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
