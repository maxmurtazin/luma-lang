---
title: "Objects"
aliases: ["/docs/objects/"]
weight: 6
---

An object is a named structural scope containing fields and methods. Methods read sibling fields lexically. Use qualified access such as Building.volume(120). No implicit receiver, instances, constructors, inheritance, first-class namespace values or arbitrary field assignment.

~~~text
object Building
    height = 25
    fn volume(area)
        return area * height
    end
end
print(Building.volume(120))
~~~

**Expected output:**

~~~text
3000
~~~
