let intervalId;
let spoolman_available = false;
let currentHexColor = null;
let rgb_disabled = true;
let saveTimeout = null;

function loadConfigAndData() {
  fetch("/config")
    .then((response) => response.json())
    .then((config) => {
      spoolman_available = config.spoolman_available;
      document.getElementById("version-span").textContent = config.version;
      if (config.color_available)
        document.getElementById("calibrate-link").classList.remove("hidden");
    })
    .catch((err) => {
      console.warn("Failed to load config:", err);
      // Fallback to loading multipliers anyway
    });
}

// Update only the display value without saving
function updateSliderDisplay(channel, value) {
  const numValue = parseFloat(value);
  rgbMult[channel] = numValue;
  document.getElementById(
    `${channel === "brightness" ? "bright" : channel}-val`,
  ).textContent = numValue.toFixed(2);
  // No updateColorDisplay here!
}

// Update and save the value (called on slider release)
function updateMultAndSave(channel, value) {
  const numValue = parseFloat(value);
  rgbMult[channel] = numValue;
  document.getElementById(
    `${channel === "brightness" ? "bright" : channel}-val`,
  ).textContent = numValue.toFixed(2);

  // Save immediately and let backend handle color update
  saveMult();
}

function updateColorDisplay() {
  if (!currentHexColor) return;
  // Only display the color as received from backend, do not apply multipliers again
  document.getElementById("color-square").style.backgroundColor =
    currentHexColor;
  document.getElementById("color-hex").textContent = currentHexColor;
}

function copyHex() {
  const hex = document.getElementById("color-hex").textContent;
  if (!hex) return;
  navigator.clipboard
    .writeText(hex)
    .then(() => {
      const el = document.getElementById("color-hex");
      const orig = el.textContent;
      el.textContent = "Copied!";
      setTimeout(() => (el.textContent = orig), 1500);
    })
    .catch(() => alert("Color: " + hex));
}

function saveToSpoolman() {
  const id = prompt("Spoolman Filament ID");
  if (!id) return;
  const val = parseFloat(document.getElementById("content").innerText);
  if (isNaN(val)) {
    alert("No valid measurement");
    return;
  }
  window.location.assign(`/spoolman/set?filament_id=${id}&value=${val}`);
}

function startPolling() {
  intervalId = setInterval(() => {
    fetch("/fallback")
      .then((response) => response.text())
      .then((data) => updateContent(data))
      .catch((err) => console.warn("Polling error:", err));
  }, 1000);
}

function updateContent(data) {
  const el = document.getElementById("content");
  const colorDisplay = document.getElementById("color-display");
  const confidenceIndicator = document.getElementById("confidence-indicator");

  if (data === "no_filament") {
    document.getElementById("save-to-spoolman-btn").classList.add("hidden");
    colorDisplay.classList.add("hidden");
    confidenceIndicator.classList.add("hidden");
    el.innerText = "No filament inserted!";
    currentHexColor = null;
    return;
  }

  const parts = data.split(",");
  if (parts.length >= 2 && parts[1].startsWith("#")) {
    const numValue = parseFloat(parts[0]);
    const hexColor = parts[1];
    const confidence = parts.length >= 3 ? parseInt(parts[2]) || 0 : 0;

    if (spoolman_available) {
      document
        .getElementById("save-to-spoolman-btn")
        .classList.remove("hidden");
    }

    el.innerText = isNaN(numValue) ? "Error!" : numValue.toFixed(2);
    colorDisplay.classList.remove("hidden");
    confidenceIndicator.classList.remove("hidden");
    currentHexColor = hexColor;
    updateColorDisplay();
    updateConfidence(confidence);
  } else {
    if (spoolman_available) {
      document
        .getElementById("save-to-spoolman-btn")
        .classList.remove("hidden");
    }
    colorDisplay.classList.add("hidden");
    confidenceIndicator.classList.add("hidden");
    currentHexColor = null;
    const num = parseFloat(data);
    el.innerText = isNaN(num) ? "Error!" : num.toFixed(2);
  }
}

function updateConfidence(sampleCount) {
  const maxSamples = 100; // Match the buffer size
  const percentage = Math.min(100, (sampleCount / maxSamples) * 100);

  document.getElementById("confidence-text").textContent =
    `${percentage.toFixed(0)}%`;
  document.getElementById("confidence-fill").style.width = `${percentage}%`;
}

loadConfigAndData();
setTimeout(startPolling, 300);

// Export functions for inline HTML event handlers
window.saveToSpoolman = saveToSpoolman;
window.copyHex = copyHex;
