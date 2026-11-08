---
title: "Начало работы"
weight: 1
---

Luma 0.1.3 включает ограниченный интерпретатор 0.1D2. Скачайте комплект Windows x64 из GitHub Releases или артефакт Actions «Windows installation distribution». Распакуйте архив перед запуском установки. В комплект входят CLI и VSIX; VS Code 1.90+ устанавливается отдельно. Сохраните исходник UTF-8 как hello.luma.

Для сборки из исходников установите stable Rust и системный компоновщик C/C++, затем выполните cargo build --release --locked. В Windows нужен Visual Studio Build Tools с компонентами разработки классических приложений C++. Запускайте target/release/luma.exe; в Linux/macOS — target/release/luma.

~~~text
print("Hello, Luma")
~~~

**Ожидаемый вывод:**

~~~text
Hello, Luma
~~~
