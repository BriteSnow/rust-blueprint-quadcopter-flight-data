// Custom tooltip for any element with a `data-tip` attribute, shown on hover and keyboard focus.

const TIP_SELECTOR = "[data-tip]";
const GAP = 10;
const EDGE = 8;

const tip = document.createElement("div");
tip.className = "tooltip";
tip.setAttribute("role", "tooltip");
const tipTitle = document.createElement("strong");
tipTitle.className = "tooltip-title";
const tipBody = document.createElement("p");
tipBody.className = "tooltip-body";
tip.append(tipTitle, tipBody);
document.body.append(tip);

let activeTarget = null;

function findTarget(event) {
  return event.target instanceof Element ? event.target.closest(TIP_SELECTOR) : null;
}

function show(target) {
  activeTarget = target;
  tipTitle.textContent = target.textContent.trim();
  tipBody.textContent = target.getAttribute("data-tip");
  tip.classList.add("is-visible");
  place(target);
}

function hide() {
  activeTarget = null;
  tip.classList.remove("is-visible");
}

/// Centers the tooltip below the target, flips it above when there is no room, and keeps it inside the viewport.
function place(target) {
  const anchor = target.getBoundingClientRect();
  const box = tip.getBoundingClientRect();
  let top = anchor.bottom + GAP;
  if (top + box.height > window.innerHeight - EDGE) {
    top = anchor.top - box.height - GAP;
  }
  const left = clamp(
    anchor.left + anchor.width / 2 - box.width / 2,
    EDGE,
    window.innerWidth - box.width - EDGE,
  );
  tip.style.transform = `translate(${Math.round(left)}px, ${Math.max(EDGE, Math.round(top))}px)`;
}

function clamp(value, min, max) {
  return Math.min(Math.max(value, min), max);
}

document.addEventListener("pointerover", (event) => {
  const target = findTarget(event);
  if (target && target !== activeTarget) {
    show(target);
  }
});

document.addEventListener("pointerout", (event) => {
  const target = findTarget(event);
  if (target && !target.contains(event.relatedTarget)) {
    hide();
  }
});

document.addEventListener("focusin", (event) => {
  const target = findTarget(event);
  if (target) {
    show(target);
  }
});

document.addEventListener("focusout", hide);
document.addEventListener("pointerdown", hide, true);
window.addEventListener("scroll", hide, true);
window.addEventListener("resize", hide);
