(() => {
    "use strict";

    const REPO_URL = "https://amqx.github.io/sources/index.min.json";

    // Show a warning if not on Apple mobile devices
    const isApple = /iphone|ipad|ipod/i.test(
        navigator.userAgent,
    );
    if (!isApple) {
        const notice = document.getElementById("platform-notice");
        if (notice) notice.hidden = false;
    }

    const copyBtn = document.getElementById("copy-btn");
    if (copyBtn) {
        let resetTimer;
        copyBtn.addEventListener("click", async () => {
            try {
                await navigator.clipboard.writeText(REPO_URL);
                copyBtn.textContent = "Copied";
            } catch {
                copyBtn.textContent = "Press ⌘/Ctrl+C";
                const range = document.createRange();
                range.selectNodeContents(document.getElementById("repo-url"));
                const selection = window.getSelection();
                selection.removeAllRanges();
                selection.addRange(range);
            }
            clearTimeout(resetTimer);
            resetTimer = setTimeout(() => {
                copyBtn.textContent = "Copy";
            }, 2000);
        });
    }

    // Source list
    const listEl = document.getElementById("sources");
    const statusEl = document.getElementById("status");
    const countEl = document.getElementById("count");
    const searchEl = document.getElementById("search");

    const DOWNLOAD_ICON =
        '<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M8 2v8m0 0 3-3m-3 3-3-3M2.5 12.5h11"/></svg>';

    // languages
    function languageLabel(languages) {
        if (!Array.isArray(languages) || languages.length === 0) return "";
        const real = languages.filter((l) => l !== "multi" && l !== "All");
        if (languages.length > real.length || real.length > 1) return "MULTI";
        return real[0].toUpperCase();
    }

    function siteLabel(baseURL) {
        if (!baseURL) return "";
        return baseURL.replace(/^https?:\/\//, "").replace(/^www\./, "").replace(/\/$/, "");
    }

    function searchText(source) {
        return [
            source.name,
            source.id,
            siteLabel(source.baseURL),
            ...(Array.isArray(source.altNames) ? source.altNames : []),
            ...(Array.isArray(source.languages) ? source.languages : []),
            languageLabel(source.languages),
        ]
            .filter(Boolean)
            .join(" ")
            .toLowerCase();
    }

    function badge(text, className, title) {
        const el = document.createElement("span");
        el.className = className ? "badge " + className : "badge";
        el.textContent = text;
        if (title) el.title = title;
        return el;
    }

    function render(source) {
        const li = document.createElement("li");
        li.className = "source";

        if (source.iconURL) {
            const icon = document.createElement("img");
            icon.className = "icon";
            icon.src = source.iconURL;
            icon.alt = "";
            icon.loading = "lazy";
            icon.decoding = "async";
            li.appendChild(icon);
        }

        const meta = document.createElement("div");
        meta.className = "meta";

        const titleRow = document.createElement("div");
        titleRow.className = "title-row";

        const name = document.createElement("span");
        name.className = "name";
        name.textContent = source.name;
        titleRow.appendChild(name);

        const lang = languageLabel(source.languages);
        if (lang) {
            titleRow.appendChild(
                badge(
                    lang,
                    "",
                    lang === "MULTI"
                        ? "Multiple languages"
                        : (source.languages || []).join(", "),
                ),
            );
        }

        if (source.contentRating === 1) {
            titleRow.appendChild(
                badge("17+", "badge-17", "Contains NSFW content"),
            );
        } else if (source.contentRating === 2) {
            titleRow.appendChild(
                badge("18+", "badge-18", "Primarily NSFW content"),
            );
        }

        meta.appendChild(titleRow);

        const sub = document.createElement("div");
        sub.className = "sub";

        const ver = document.createElement("span");
        ver.className = "ver";
        ver.textContent = "v" + source.version;
        sub.appendChild(ver);

        const site = siteLabel(source.baseURL);
        if (site) {
            const dot = document.createElement("span");
            dot.textContent = "·";
            dot.setAttribute("aria-hidden", "true");
            sub.appendChild(dot);

            const siteEl = document.createElement("a");
            siteEl.className = "site";
            siteEl.href = source.baseURL;
            siteEl.target = "_blank";
            siteEl.rel = "noopener noreferrer";
            siteEl.title = source.baseURL;
            siteEl.textContent = site;
            sub.appendChild(siteEl);
        }

        meta.appendChild(sub);
        li.appendChild(meta);

        const dl = document.createElement("a");
        dl.className = "dl";
        dl.href = source.downloadURL;
        dl.setAttribute("download", "");
        dl.title = "Download " + source.name + " v" + source.version;
        dl.setAttribute(
            "aria-label",
            "Download " + source.name + " v" + source.version,
        );
        dl.innerHTML = DOWNLOAD_ICON;
        li.appendChild(dl);

        return li;
    }

    let entries = [];

    function applyFilter() {
        const query = searchEl.value.trim().toLowerCase();
        const terms = query.split(/\s+/).filter(Boolean);
        let visible = 0;
        let lastShown = null;

        for (const entry of entries) {
            const show = terms.every((term) => entry.haystack.includes(term));
            entry.li.hidden = !show;
            entry.li.classList.remove("last-visible");
            if (show) {
                visible++;
                lastShown = entry.li;
            }
        }

        if (lastShown) lastShown.classList.add("last-visible");

        countEl.textContent = query
            ? visible + " of " + entries.length
            : String(entries.length);

        if (visible === 0) {
            statusEl.hidden = false;
            statusEl.textContent = "No sources match “" + searchEl.value.trim() + "”.";
        } else {
            statusEl.hidden = true;
        }
    }

    fetch("index.min.json")
        .then((response) => {
            if (!response.ok) throw new Error("HTTP " + response.status);
            return response.json();
        })
        .then((data) => {
            const sources = Array.isArray(data.sources) ? data.sources : [];
            if (sources.length === 0) {
                statusEl.textContent = "No sources published yet.";
                return;
            }

            sources.sort((a, b) =>
                a.name.localeCompare(b.name, undefined, {sensitivity: "base"}),
            );

            const fragment = document.createDocumentFragment();
            entries = sources.map((source) => {
                const li = render(source);
                fragment.appendChild(li);
                return {li, haystack: searchText(source)};
            });
            listEl.appendChild(fragment);

            searchEl.addEventListener("input", applyFilter);
            applyFilter();
        })
        .catch((error) => {
            statusEl.hidden = false;
            statusEl.textContent =
                "Could not load the source list. Try reloading the page.";
            console.error(error);
        });
})();
