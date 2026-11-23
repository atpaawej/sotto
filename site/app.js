const copyBtn = document.getElementById("copy-btn");
const installCmd = document.getElementById("install-cmd");
const copyStatus = document.getElementById("copy-status");

if (copyBtn && installCmd) {
  let resetTimer;

  copyBtn.addEventListener("click", async () => {
    const commandText = installCmd.textContent?.trim() ?? "";
    if (!commandText) return;

    try {
      await navigator.clipboard.writeText(commandText);
      copyBtn.textContent = "Copied";
      if (copyStatus) {
        copyStatus.textContent = "Install command copied to clipboard.";
      }
    } catch {
      copyBtn.textContent = "Select & copy";
      if (copyStatus) {
        copyStatus.textContent = "Clipboard access unavailable. Please copy the command manually.";
      }
    }

    window.clearTimeout(resetTimer);
    resetTimer = window.setTimeout(() => {
      copyBtn.textContent = "Copy";
    }, 2000);
  });
}
