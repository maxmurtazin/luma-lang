---
title: "Установка Windows"
weight: 2
---

Установка выполняется для текущего пользователя в %LOCALAPPDATA%\Programs\Luma без прав администратора и изменения PATH. В PowerShell выполните & "$env:LOCALAPPDATA\Programs\Luma\bin\luma.exe" version. Используйте полный путь для check/run или задайте luma.executablePath в VS Code. Расширение с настройкой по умолчанию находит этот каталог.

Выберите дополнительную задачу VS Code для установки комплектного VSIX, затем перезагрузите редактор. Если VS Code отсутствует или произошла ошибка, прочитайте extension-install.log и выберите Extensions: Install from VSIX для файла luma-language-tools-0.1.3.vsix в каталоге установки. Отдельный VSIX не содержит интерпретатор.

Для обновления повторно запустите установщик, предварительно завершив процессы Luma. Удаление доступно в списке установленных приложений Windows. Удаляются только файлы установщика; пользовательские файлы и отдельно зарегистрированное расширение VS Code сохраняются. Расширение можно удалить в VS Code. CI проверяет повторную установку, сохранение пользовательского файла, CLI, экспорт графа, регистрацию VSIX и удаление в отдельной учётной записи Windows. Повторная установка не проверяет миграцию со старой версии.


[Скачать последний установщик Windows x64](https://github.com/maxmurtazin/luma-lang/releases/latest/download/Luma-windows-x64-setup.exe). [VSIX](https://github.com/maxmurtazin/luma-lang/releases/latest/download/luma-language-tools.vsix). [Все файлы выпуска и контрольные суммы](https://github.com/maxmurtazin/luma-lang/releases/latest).

CI также устанавливает опубликованную версию 0.1.2 и проверяет обновление до 0.1.3 с сохранением пользовательского файла.
