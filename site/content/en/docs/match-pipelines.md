---
title: "Match and pipelines"
aliases: ["/docs/match-pipelines/"]
weight: 8
---

x -> f(a) means f(x, a). Input evaluates before explicit arguments; stages run sequentially. Bare f calls with only the input. match checks literal/comparison patterns in order, runs only the first selected expression and requires a final * fallback. All result types must unify. Ordered patterns require Number; literal patterns support Number, String and Bool.

~~~text
fn add(x, y)
    return x + y
end
value = 4 -> add(2)
print(match value
    > 5: "large"
    *: "small"
end)
~~~

**Expected output:**

~~~text
large
~~~
