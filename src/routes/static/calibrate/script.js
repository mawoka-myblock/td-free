let intervalId;
let currentHexColor = null;
let rgb_disabled = true;
let rgbMult = {
  red: 1.0,
  green: 1.0,
  blue: 1.0,
  brightness: 1.0,
  td_reference: 50.0,
  reference_r: 127,
  reference_g: 127,
  reference_b: 127,
};
let saveTimeout = null;

async function loadMult() {
  const res = await fetch("/rgb_multipliers");
  const data = await response.json();
  if (data.red !== undefined) rgbMult.red = data.red;
  if (data.green !== undefined) rgbMult.green = data.green;
  if (data.blue !== undefined) rgbMult.blue = data.blue;
  if (data.brightness !== undefined) rgbMult.brightness = data.brightness;
  if (data.td_reference !== undefined) rgbMult.td_reference = data.td_reference;
  if (data.reference_r !== undefined) rgbMult.reference_r = data.reference_r;
  if (data.reference_g !== undefined) rgbMult.reference_g = data.reference_g;
  if (data.reference_b !== undefined) rgbMult.reference_b = data.reference_b;
  rgb_disabled = data.rgb_disabled;
  updateSliders();
  updateRefDisplay();
}

function updateSliders() {
  document.getElementById("bright-val").textContent =
    rgbMult.brightness.toFixed(2);
  if (rgb_disabled) {
    document.getElementsByClassName("rgb-control");
  }
  document.getElementById("red-mult").value = rgbMult.red.toFixed(2);
  document.getElementById("green-mult").value = rgbMult.green.toFixed(2);
  document.getElementById("blue-mult").value = rgbMult.blue.toFixed(2);
  document.getElementById("bright-mult").value = rgbMult.brightness.toFixed(2);
  document.getElementById("red-val").textContent = rgbMult.red.toFixed(2);
  document.getElementById("green-val").textContent = rgbMult.green.toFixed(2);
  document.getElementById("blue-val").textContent = rgbMult.blue.toFixed(2);
}

function updateRefDisplay() {
  const r = rgbMult.reference_r;
  const g = rgbMult.reference_g;
  const b = rgbMult.reference_b;
  const hex = `#${r.toString(16).padStart(2, "0")}${g.toString(16).padStart(2, "0")}${b.toString(16).padStart(2, "0")}`;
  document.getElementById("ref-color-square").style.backgroundColor = hex;
  document.getElementById("ref-hex").textContent = hex.toUpperCase();
}

function toggleCalibrationBlock() {
  const content = document.getElementById("calibration-content");
  const header = content.previousElementSibling;
  content.classList.toggle("hidden");
  header.classList.toggle("open");
}

function openColorPickerModal() {
  // Set sliders to current reference color
  document.getElementById("picker-r").value = rgbMult.reference_r;
  document.getElementById("picker-g").value = rgbMult.reference_g;
  document.getElementById("picker-b").value = rgbMult.reference_b;
  updatePickerPreview();
  document.getElementById("color-picker-modal").style.display = "flex";
}

function closeColorPickerModal() {
  document.getElementById("color-picker-modal").style.display = "none";
}

function updatePickerPreview() {
  const r = parseInt(document.getElementById("picker-r").value);
  const g = parseInt(document.getElementById("picker-g").value);
  const b = parseInt(document.getElementById("picker-b").value);
  const hex = `#${r.toString(16).padStart(2, "0")}${g.toString(16).padStart(2, "0")}${b.toString(16).padStart(2, "0")}`;
  document.getElementById("picker-preview").style.backgroundColor = hex;
  document.getElementById("picker-hex").textContent = hex.toUpperCase();
  document.getElementById("picker-r-val").value = r;
  document.getElementById("picker-g-val").value = g;
  document.getElementById("picker-b-val").value = b;
}

function updateSliderFromInput(input, channel) {
  let value = parseInt(input.value, 10);
  if (isNaN(value)) return;

  // Clamp the value between 0 and 255
  value = Math.max(0, Math.min(255, value));

  document.getElementById(`picker-${channel}`).value = value;
  updatePickerPreview();
}

function updateFromNativePicker(hex) {
  const r = parseInt(hex.substring(1, 3), 16);
  const g = parseInt(hex.substring(3, 5), 16);
  const b = parseInt(hex.substring(5, 7), 16);
  document.getElementById("picker-r").value = r;
  document.getElementById("picker-g").value = g;
  document.getElementById("picker-b").value = b;
  updatePickerPreview();
}

function applyPickerColor() {
  const r = parseInt(document.getElementById("picker-r").value);
  const g = parseInt(document.getElementById("picker-g").value);
  const b = parseInt(document.getElementById("picker-b").value);
  rgbMult.reference_r = r;
  rgbMult.reference_g = g;
  rgbMult.reference_b = b;
  updateRefDisplay();
  saveMult();
  closeColorPickerModal();
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

function saveMult() {
  fetch("/rgb_multipliers", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(rgbMult),
  }).catch((err) => console.warn("Save failed:", err));
}

function autoCal() {
  const button = document.getElementById("cal-btn");
  button.disabled = true;
  button.textContent = "Calibrating...";

  fetch("/auto_calibrate", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      reference_r: rgbMult.reference_r,
      reference_g: rgbMult.reference_g,
      reference_b: rgbMult.reference_b,
    }),
  })
    .then((response) => response.json())
    .then((data) => {
      if (data.status === "success") {
        rgbMult.red = data.red;
        rgbMult.green = data.green;
        rgbMult.blue = data.blue;
        rgbMult.brightness = data.brightness;
        if (data.td_reference !== undefined) {
          rgbMult.td_reference = data.td_reference;
        }
        updateSliders();
        updateColorDisplay();
        button.textContent = "Success!";
      } else {
        alert("Calibration failed: " + (data.message || "Unknown error"));
        button.textContent = "Auto-Calibrate";
      }
      setTimeout(() => {
        button.textContent = "Auto-Calibrate";
        button.disabled = false;
      }, 2000);
    })
    .catch((err) => {
      console.warn("Calibration failed:", err);
      alert("Calibration failed. Please try again.");
      button.textContent = "Auto-Calibrate";
      button.disabled = false;
    });
}

function resetMult() {
  rgbMult = {
    red: 1.0,
    green: 1.0,
    blue: 1.0,
    brightness: 1.0,
    td_reference: 50.0,
    reference_r: 127,
    reference_g: 127,
    reference_b: 127,
  };
  updateSliders();
  updateRefDisplay();
  saveMult();
  updateColorDisplay();
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

    el.innerText = isNaN(numValue) ? "Error!" : numValue.toFixed(2);
    colorDisplay.classList.remove("hidden");
    confidenceIndicator.classList.remove("hidden");
    currentHexColor = hexColor;
    updateColorDisplay();
    updateConfidence(confidence);
  } else {
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

setTimeout(startPolling, 300);

// Export functions for inline HTML event handlers
window.updateSliderDisplay = updateSliderDisplay;
window.updateMultAndSave = updateMultAndSave;
window.toggleCalibrationBlock = toggleCalibrationBlock;
window.openColorPickerModal = openColorPickerModal;
window.closeColorPickerModal = closeColorPickerModal;
window.updateSliderFromInput = updateSliderFromInput;
window.applyPickerColor = applyPickerColor;
window.updateFromNativePicker = updateFromNativePicker;
window.autoCal = autoCal;
window.resetMult = resetMult;
window.copyHex = copyHex;
