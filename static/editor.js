document.addEventListener("DOMContentLoaded", () => {
  document.querySelectorAll('form[action$="/delete"]').forEach((form) => {
    form.addEventListener("submit", (event) => {
      if (!window.confirm("Delete this item? This cannot be undone.")) {
        event.preventDefault();
      }
    });
  });

  document.querySelectorAll(".markdown-editor").forEach((textarea) => {
    const endpoint = textarea.dataset.imageUpload;
    new EasyMDE({
      element: textarea,
      autoDownloadFontAwesome: false,
      forceSync: true,
      minHeight: "360px",
      previewImagesInEditor: true,
      uploadImage: Boolean(endpoint),
      imageUploadEndpoint: endpoint || undefined,
      imagePathAbsolute: true,
      imageMaxSize: 10 * 1024 * 1024,
      imageAccept: "image/png,image/jpeg,image/webp,image/gif",
      toolbar: [
        "bold",
        "italic",
        "heading",
        "|",
        "quote",
        "unordered-list",
        "ordered-list",
        "link",
        "image",
        ...(endpoint ? ["upload-image"] : []),
        "|",
        "preview",
        "side-by-side",
        "fullscreen",
        "|",
        "guide",
      ],
    });

    if (endpoint) {
      const helperForm = document.createElement("form");
      helperForm.id = `markdown-image-upload-${Math.random().toString(36).slice(2)}`;
      helperForm.method = "post";
      helperForm.enctype = "multipart/form-data";
      helperForm.hidden = true;
      document.body.appendChild(helperForm);
      textarea
        .closest("form")
        .querySelectorAll('input[type="file"]')
        .forEach((input) => input.setAttribute("form", helperForm.id));
    }
  });

  document.querySelectorAll(".copy-image").forEach((button) => {
    button.addEventListener("click", async () => {
      await navigator.clipboard.writeText(button.dataset.copy);
      button.textContent = "Copied";
      setTimeout(() => {
        button.textContent = "Copy URL";
      }, 1200);
    });
  });

  document.querySelectorAll(".image-card form").forEach((form) => {
    form.addEventListener("submit", async (event) => {
      if (event.defaultPrevented) return;
      event.preventDefault();
      const response = await fetch(form.action, { method: "POST" });
      if (response.ok) {
        form.closest(".image-card").remove();
      }
    });
  });
});
