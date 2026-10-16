//js/notifications.js
// Toast notifications. Extracted from the removed downloads module; the DOM
// ids/classes are unchanged so existing styles keep applying.

let notificationContainer = null;

function getNotificationContainer() {
    if (!notificationContainer) {
        notificationContainer = document.createElement('div');
        notificationContainer.id = 'download-notifications';
        document.body.appendChild(notificationContainer);
    }
    return notificationContainer;
}

export function showNotification(message, options = {}) {
    if (typeof options === 'string') options = { type: options };
    const container = getNotificationContainer();

    const notifEl = document.createElement('div');
    notifEl.className = 'download-task';

    const innerDiv = document.createElement('div');
    innerDiv.className = 'notification-message';
    innerDiv.textContent = message;
    notifEl.appendChild(innerDiv);

    const isError =
        options.type === 'error' ||
        /\b(error|failed|failure|could not|couldn't|cannot|unable|unavailable|not found)\b/i.test(String(message)) ||
        // \b does not work for Cyrillic.
        /(не удалось|ошибка|недоступ|не найден|не воспроизв|не отдаёт)/i.test(String(message));

    container.appendChild(notifEl);

    setTimeout(
        () => {
            notifEl.style.animation = 'slide-out 0.3s ease forwards';
            setTimeout(() => notifEl.remove(), 300);
        },
        isError ? 10000 : 1500
    );
}
