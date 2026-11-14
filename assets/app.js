// Small progressive enhancements. Everything works without this file.
(function () {
  "use strict";

  function modal() {
    return document.getElementById("modal");
  }

  // ---------------------------------------------------------------- tag editor
  // Turns <input data-tags data-suggestions="a,b"> into chips with
  // autosuggest and a "Create" option. The original input keeps the
  // comma-separated value, so the form posts exactly what it would without JS.

  var TAG_RE = /^[\p{L}\p{N} ._-]{1,32}$/u;
  var MAX_TAGS = 10;
  var uid = 0;

  function normalize(t) {
    return t.trim().replace(/\s+/g, " ").toLowerCase();
  }

  function el(tag, cls, text) {
    var e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text != null) e.textContent = text;
    return e;
  }

  function enhanceTags(input) {
    if (input.dataset.tagsReady) return;
    input.dataset.tagsReady = "1";
    var id = "tags-" + (++uid);
    var suggestions = (input.dataset.suggestions || "").split(",").map(normalize).filter(Boolean);
    var tags = input.value.split(/[,;]/).map(normalize).filter(Boolean);
    var active = -1;
    var options = [];

    var box = el("div", "form-control d-flex flex-wrap align-items-center gap-1 h-auto");
    var chips = el("span", "kurtz-contents");
    var entry = el("input", "flex-fill border-0 bg-transparent p-0");
    entry.type = "text";
    entry.style.outline = "none";
    entry.style.minWidth = "8rem";
    entry.id = input.id;
    entry.autocomplete = "off";
    entry.setAttribute("role", "combobox");
    entry.setAttribute("aria-autocomplete", "list");
    entry.setAttribute("aria-expanded", "false");
    entry.setAttribute("aria-controls", id + "-list");
    entry.placeholder = input.placeholder;
    var list = el("ul", "kurtz-suggest position-absolute start-0 end-0 shadow d-none");
    list.id = id + "-list";
    list.setAttribute("role", "listbox");
    var wrap = el("div", "position-relative");

    input.removeAttribute("id");
    input.type = "hidden";
    input.parentNode.insertBefore(wrap, input);
    wrap.appendChild(box);
    wrap.appendChild(list);
    box.appendChild(chips);
    box.appendChild(entry);
    box.addEventListener("click", function () { entry.focus(); });

    function sync() {
      input.value = tags.join(", ");
      chips.innerHTML = "";
      tags.forEach(function (t, i) {
        var li = el("span", "badge bg-primary-lt d-inline-flex align-items-center gap-1");
        li.appendChild(el("span", "", "#" + t));
        var x = el("button", "btn-close");
        x.type = "button";
        x.setAttribute("aria-label", "Remove tag " + t);
        x.addEventListener("click", function (e) {
          e.stopPropagation();
          tags.splice(i, 1);
          sync();
          entry.focus();
        });
        li.appendChild(x);
        chips.appendChild(li);
      });
      entry.disabled = tags.length >= MAX_TAGS;
      entry.placeholder = tags.length ? "" : input.placeholder;
    }

    function add(t) {
      t = normalize(t);
      if (!TAG_RE.test(t) || tags.indexOf(t) !== -1 || tags.length >= MAX_TAGS) return false;
      tags.push(t);
      if (suggestions.indexOf(t) === -1) suggestions.push(t);
      entry.value = "";
      sync();
      close();
      return true;
    }

    function close() {
      list.classList.add("d-none");
      entry.setAttribute("aria-expanded", "false");
      entry.removeAttribute("aria-activedescendant");
      active = -1;
    }

    function render() {
      var q = normalize(entry.value);
      var matches = suggestions.filter(function (s) {
        return tags.indexOf(s) === -1 && (!q || s.indexOf(q) !== -1);
      }).sort(function (a, b) {
        return (b.indexOf(q) === 0) - (a.indexOf(q) === 0) || a.localeCompare(b);
      }).slice(0, 8);
      options = matches.map(function (m) { return { value: m, label: "#" + m }; });
      if (q && suggestions.indexOf(q) === -1 && tags.indexOf(q) === -1) {
        options.push({ value: q, label: TAG_RE.test(q) ? "Create #" + q : "Not allowed: " + q, create: true, invalid: !TAG_RE.test(q) });
      }
      list.innerHTML = "";
      if (!options.length) return close();
      options.forEach(function (o, i) {
        var li = el("li", o.invalid ? "text-danger" : o.create ? "fw-bold" : "", o.label);
        li.id = id + "-opt-" + i;
        li.setAttribute("role", "option");
        li.addEventListener("mousedown", function (e) {
          e.preventDefault();
          if (!o.invalid) add(o.value);
        });
        list.appendChild(li);
      });
      active = q ? 0 : -1;
      highlight();
      list.classList.remove("d-none");
      entry.setAttribute("aria-expanded", "true");
    }

    function highlight() {
      Array.prototype.forEach.call(list.children, function (li, i) {
        var on = i === active;
        li.setAttribute("aria-selected", on ? "true" : "false");
        li.classList.toggle("active", on);
      });
      if (active >= 0) entry.setAttribute("aria-activedescendant", id + "-opt-" + active);
      else entry.removeAttribute("aria-activedescendant");
    }

    entry.addEventListener("input", function () {
      if (/[,;]/.test(entry.value)) {
        entry.value.split(/[,;]/).forEach(function (part, i, all) {
          if (i < all.length - 1) add(part);
          else entry.value = part;
        });
      }
      render();
    });
    entry.addEventListener("focus", render);
    entry.addEventListener("blur", function () {
      if (normalize(entry.value)) add(entry.value);
      close();
    });
    entry.addEventListener("keydown", function (e) {
      var open = !list.classList.contains("d-none");
      if (e.key === "ArrowDown" && open) {
        e.preventDefault();
        active = Math.min(active + 1, options.length - 1);
        highlight();
      } else if (e.key === "ArrowUp" && open) {
        e.preventDefault();
        active = Math.max(active - 1, 0);
        highlight();
      } else if (e.key === "Enter") {
        if (!entry.value.trim() && active < 0) return; // submit the form
        e.preventDefault();
        var o = options[active];
        if (o && !o.invalid) add(o.value);
        else add(entry.value);
      } else if (e.key === "Escape" && open) {
        e.preventDefault();
        e.stopPropagation();
        close();
      } else if (e.key === "Backspace" && !entry.value && tags.length) {
        tags.pop();
        sync();
        render();
      }
    });

    sync();
  }

  // Shows <time datetime="…" data-time="date|datetime"> in the visitor's locale
  // and time zone; the server-rendered text (UTC) is the fallback.
  var DATE_FORMATS = {
    date: { dateStyle: "medium" },
    datetime: { dateStyle: "medium", timeStyle: "short" }
  };

  function formatTimes() {
    document.querySelectorAll("time[data-time]").forEach(function (t) {
      var d = new Date(t.getAttribute("datetime"));
      if (isNaN(d.getTime())) return;
      t.textContent = new Intl.DateTimeFormat(undefined, DATE_FORMATS[t.dataset.time] || DATE_FORMATS.datetime).format(d);
      t.title = d.toLocaleString();
    });
  }

  function init(root) {
    formatTimes();
    (root || document).querySelectorAll("input[data-tags]").forEach(enhanceTags);
  }

  document.addEventListener("DOMContentLoaded", function () { init(); });

  // ---------------------------------------------------------------- modal

  // Open the dialog once htmx has loaded a form into it.
  document.addEventListener("htmx:afterSwap", function (event) {
    init(event.detail.target);
    if (event.detail.target && event.detail.target.id === "modal-body") {
      var dialog = modal();
      if (dialog && !dialog.open) dialog.showModal();
      var first = dialog && dialog.querySelector("input:not([type=hidden]):not([type=checkbox]):not([disabled])");
      if (first) first.focus();
    }
  });

  document.addEventListener("click", function (event) {
    var copy = event.target.closest("[data-copy]");
    if (copy && navigator.clipboard) {
      event.preventDefault();
      navigator.clipboard.writeText(copy.getAttribute("data-copy")).then(function () {
        var label = copy.textContent;
        copy.textContent = "Copied";
        setTimeout(function () { copy.textContent = label; }, 1200);
      });
      return;
    }
    if (event.target.closest("[data-close-modal]")) {
      event.preventDefault();
      var dialog = modal();
      if (dialog) dialog.close();
      return;
    }
    // Clicking the backdrop closes the dialog.
    var d = modal();
    if (d && event.target === d) d.close();
  });
})();
