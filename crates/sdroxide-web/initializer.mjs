// Trunk wasm-load hook: progress UI while the ~30 MB client downloads.
// See https://trunkrs.dev/guide/advanced/initializer.html

function $(id) {
  return document.getElementById(id);
}

function showOverlay() {
  const el = $("sdroxide-loader");
  if (el) el.hidden = false;
}

function hideOverlay() {
  const el = $("sdroxide-loader");
  if (el) el.hidden = true;
}

function setStatus(text) {
  const el = $("sdroxide-loader-status");
  if (el) el.textContent = text;
}

function setProgress(current, total) {
  const bar = $("sdroxide-loader-bar");
  const label = $("sdroxide-loader-pct");
  if (!bar) return;
  if (total > 0) {
    const pct = Math.min(100, Math.round((current / total) * 100));
    bar.style.width = `${pct}%`;
    bar.classList.remove("indeterminate");
    if (label) label.textContent = `${pct}%`;
    const mb = (current / (1024 * 1024)).toFixed(1);
    const tot = (total / (1024 * 1024)).toFixed(1);
    setStatus(`Loading client… ${mb} / ${tot} MB`);
  } else {
    bar.classList.add("indeterminate");
    bar.style.width = "40%";
    if (label) label.textContent = "";
    const mb = (current / (1024 * 1024)).toFixed(1);
    setStatus(current > 0 ? `Loading client… ${mb} MB` : "Loading client…");
  }
}

function showFailure(error) {
  showOverlay();
  setStatus("Failed to load the client");
  const err = $("sdroxide-loader-error");
  if (err) {
    err.hidden = false;
    err.textContent = error != null ? String(error) : "Unknown error";
  }
  const retry = $("sdroxide-loader-retry");
  if (retry) retry.hidden = false;
}

export default function sdroxideInitializer() {
  return {
    onStart: () => {
      showOverlay();
      setProgress(0, 0);
    },
    onProgress: ({ current, total }) => {
      setProgress(current, total);
    },
    onComplete: () => {},
    onSuccess: () => {
      hideOverlay();
    },
    onFailure: (error) => {
      showFailure(error);
    },
  };
}
