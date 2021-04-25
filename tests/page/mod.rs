use super::Which;
use futures::stream::StreamExt;
use playwright::api::{page, BrowserContext, Geolocation, Page, Viewport};

pub async fn all(c: &BrowserContext, port: u16, which: Which) {
    let page = c.new_page().await.unwrap();
    eq_context_close(c, &page).await;
    ensure_timeout(&page).await;
    set_timeout(&page).await;
    permissions(c, &page, port, which).await;
    if which != Which::Firefox {
        // XXX: go_back response is null on firefox
        navigations(&page, port).await;
    }
    front_should_work(c, &page).await;
    tokio::join!(
        set_extra_http_headers(c, port),
        focus_should_work(c),
        reload_should_worker(c),
        screenshot_should_work(&page),
        title_should_work(&page),
        check_should_work(c),
        pointer(c),
        viewport(c),
        download(c, port),
        workers_should_work(c, port, which),
        accessibility(c)
    );
    // query_selector_and_eval(&page).await;
    if which != Which::Firefox {
        pdf_should_work(&page).await;
    }
    video(&page).await;
    // emulate(&page).await;
    text(&page).await;
}

async fn eq_context_close(c: &BrowserContext, p1: &Page) {
    let p2 = new(c).await;
    assert_ne!(p1, &p2);
    assert_eq!(&p1.context(), c);
    assert_eq!(&p2.context(), c);
    ensure_close(&p2).await;
}

async fn ensure_close(page: &Page) {
    let mut rx = page.subscribe_event().unwrap();
    let receive_close = async {
        let mut received = false;
        while let Some(Ok(evt)) = rx.next().await {
            if let page::Event::Close = evt {
                received = true;
                break;
            }
        }
        received
    };
    let (received, wait_result, result) = tokio::join!(
        receive_close,
        page.expect_event(page::EventType::Close),
        page.close(None)
    );
    result.unwrap();
    assert_eq!(received, true);
    match wait_result.unwrap() {
        page::Event::Close => (),
        _ => unreachable!()
    }
}

async fn front_should_work(c: &BrowserContext, p1: &Page) {
    let p2 = new(c).await;
    p1.bring_to_front().await.unwrap();
    assert_eq!(
        p1.eval::<String>("document.visibilityState").await.unwrap(),
        "visible"
    );
    assert_eq!(
        p2.eval::<String>("document.visibilityState").await.unwrap(),
        "visible"
    );
    close(&p2).await;
}

async fn focus_should_work(c: &BrowserContext) {
    let page = new(c).await;
    page.set_content_builder("<div id=d1 tabIndex=0></div>")
        .set_content()
        .await
        .unwrap();
    assert_eq!(
        page.eval::<String>("() => document.activeElement.nodeName")
            .await
            .unwrap(),
        "BODY"
    );
    page.focus("#d1", None).await.unwrap();
    assert_eq!(
        page.eval::<String>("(s) => document.activeElement.id")
            .await
            .unwrap(),
        "d1"
    );
    close(&page).await;
}

async fn reload_should_worker(c: &BrowserContext) {
    let page = new(c).await;
    page.evaluate::<i32, i32>("x => window._foo = x", 10)
        .await
        .unwrap();
    page.reload_builder().reload().await.unwrap();
    let x: Option<i32> = page.eval("() => window._foo").await.unwrap();
    assert_eq!(x, None);
    close(&page).await;
}

async fn navigations(page: &Page, port: u16) {
    assert_eq!(page.go_back_builder().go_back().await.unwrap(), None);
    let url1 = super::url_static(port, "/empty.html");
    let url2 = super::url_static(port, "/empty2.html");
    page.goto_builder(&url1).goto().await.unwrap();
    page.goto_builder(&url2).goto().await.unwrap();
    {
        let response = page.go_back_builder().go_back().await.unwrap().unwrap();
        assert_eq!(response.ok().unwrap(), true);
        assert_eq!(response.url().unwrap(), url1);
    }
    {
        let response = page
            .go_forward_builder()
            .go_forward()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(response.ok().unwrap(), true);
        assert_eq!(response.url().unwrap(), url2);
    }
    let maybe_response = page.go_forward_builder().go_forward().await.unwrap();
    assert_eq!(maybe_response, None);
}

async fn set_timeout(page: &Page) {
    page.set_default_navigation_timeout(10000).await.unwrap();
    page.set_default_timeout(10000).await.unwrap();
}

async fn workers_should_work(c: &BrowserContext, port: u16, which: Which) {
    let page = new(c).await;
    let url = super::url_static(port, "/worker.html");
    let js = super::url_static(port, "/worker.js");
    let empty = super::url_static(port, "/empty.html");
    let workers = || page.workers().unwrap();
    assert_eq!(workers().len(), 0);
    let (_, _) = tokio::join!(
        page.expect_event(page::EventType::Worker),
        page.goto_builder(&url).goto()
    );
    assert_eq!(workers().len(), 1);
    let w = &workers()[0];
    assert_eq!(
        w.url().unwrap(),
        match which {
            Which::Firefox => "worker.js".to_owned(),
            _ => js
        }
    );
    assert_eq!(
        w.eval::<String>("() => self.workerFunction()")
            .await
            .unwrap(),
        "worker function result"
    );
    page.goto_builder(&empty).goto().await.unwrap();
    assert_eq!(workers().len(), 0);
    close(&page).await;
}

async fn ensure_timeout(page: &Page) {
    page.set_default_timeout(500).await.unwrap();
    match page.expect_event(page::EventType::Load).await {
        Err(playwright::Error::Timeout) => {}
        _ => panic!("Not expected")
    }
}

async fn permissions(c: &BrowserContext, page: &Page, port: u16, which: Which) {
    const PERMISSION_DENIED: i32 = 1;
    let snippet = "async () => {
        let getCurrentPositionAsync =
            () => new Promise((resolve, reject) =>
                navigator.geolocation.getCurrentPosition(resolve, reject));
        let err;
        const result = await getCurrentPositionAsync().catch(e => { err = e; });
        return [result?.coords.latitude, err?.code];
    }";
    page.goto_builder(&super::url_static(port, "/empty.html"))
        .goto()
        .await
        .unwrap();
    let geo = || async {
        page.eval::<(Option<f64>, Option<i32>)>(snippet)
            .await
            .unwrap()
    };
    assert_eq!(get_permission(page, "geolocation").await, "granted");
    c.clear_permissions().await.unwrap();
    assert_eq!(get_permission(page, "geolocation").await, "prompt");
    if which != Which::Firefox {
        // firefox shows prompt
        assert_eq!(geo().await, (None, Some(PERMISSION_DENIED)));
    }
    c.grant_permissions(&["geolocation".into()], None)
        .await
        .unwrap();
    assert_eq!(get_permission(page, "geolocation").await, "granted");
    c.set_geolocation(Some(&Geolocation {
        latitude: 59.95,
        longitude: 2.,
        accuracy: None
    }))
    .await
    .unwrap();
    let result = geo().await;
    dbg!(&result);
    assert_eq!(result.0, Some(59.95))
}

async fn get_permission(p: &Page, name: &str) -> String {
    p.evaluate(
        "name => navigator.permissions.query({name}).then(result => result.state)",
        name
    )
    .await
    .unwrap()
}

async fn viewport(c: &BrowserContext) {
    let p = new(c).await;
    let v = Viewport {
        width: 500,
        height: 500
    };
    dbg!(p.viewport_size().unwrap());
    p.set_viewport_size(v.clone()).await.unwrap();
    assert_eq!(p.viewport_size().unwrap(), Some(v));
    close(&p).await;
}

async fn download(c: &BrowserContext, port: u16) {
    let p = new(c).await;
    p.set_content_builder(&format!(
        r#"<a href="{}">download</a>"#,
        super::url_download(port, "/worker.html")
    ))
    .set_content()
    .await
    .unwrap();
    let (d, _) = tokio::join!(
        p.expect_event(page::EventType::Download),
        p.click_builder("a").click()
    );
    let download = match d.unwrap() {
        page::Event::Download(d) => d,
        _ => unreachable!()
    };
    dbg!(download.url());
    dbg!(download.suggested_filename());
    dbg!(download.path().await.unwrap());
    assert!(!download.url().is_empty());
    assert!(!download.suggested_filename().is_empty());
    assert!(download.path().await.unwrap().is_some());
    assert_eq!(download.failure().await.unwrap(), None);
    let tmp = super::temp_dir().join(download.suggested_filename());
    download.save_as(tmp).await.unwrap();
    download.delete().await.unwrap();
    close(&p).await;
}

async fn video(p: &Page) {
    let video = p.video().unwrap().unwrap();
    dbg!(video.path().unwrap());
    // TODO
    // let path = super::temp_dir().join("video.webm");
    // video.save_as(&path).await.unwrap();
    // assert!(path.is_file());
    // video.delete().await.unwrap();
}

async fn accessibility(c: &BrowserContext) {
    let p = new(c).await;
    use playwright::api::accessibility::SnapshotResponse;
    let ac = &p.accessibility;
    p.set_content_builder(
        r#"<div>\
            <span>Hello World</span>\
            <input placeholder="Empty input" autofocus />\
        </div>"#
    )
    .set_content()
    .await
    .unwrap();
    let span = p.query_selector("span").await.unwrap().unwrap();
    let input = p.query_selector("input").await.unwrap().unwrap();
    let snapshot = ac
        .snapshot_builder()
        .try_root(input)
        .unwrap()
        .snapshot()
        .await
        .unwrap();
    let input_response = Some(SnapshotResponse {
        role: "textbox".into(),
        name: "Empty input".into(),
        value: None,
        description: None,
        keyshortcuts: None,
        roledescription: None,
        valuetext: None,
        disabled: None,
        expanded: None,
        focused: Some(true),
        modal: None,
        multiline: None,
        multiselectable: None,
        readonly: None,
        required: None,
        selected: None,
        checked: None,
        pressed: None,
        level: None,
        valuemin: None,
        valuemax: None,
        autocomplete: None,
        haspopup: None,
        invalid: None,
        orientation: None,
        children: Vec::new()
    });
    assert_eq!(snapshot, input_response);
    let snapshot = ac
        .snapshot_builder()
        .try_root(span)
        .unwrap()
        .clear_root()
        .interesting_only(false)
        .snapshot()
        .await
        .unwrap();
    assert_ne!(snapshot, input_response);
    close(&p).await;
}

async fn screenshot_should_work(p: &Page) {
    use playwright::api::ScreenshotType;
    let path = super::temp_dir().join("screenshot.jpg");
    p.screenshot_builder()
        .r#type(ScreenshotType::Jpeg)
        .clear_type()
        .path(path.clone())
        .screenshot()
        .await
        .unwrap();
    assert!(path.is_file());
}

async fn pdf_should_work(p: &Page) {
    let path = super::temp_dir().join("pdf.pdf");
    p.pdf_builder().path(path.clone()).pdf().await.unwrap();
    assert!(path.is_file());
}

async fn emulate(p: &Page) {
    use playwright::api::page::Media;
    let screen = || async {
        p.eval::<bool>("() => matchMedia('screen').matches")
            .await
            .unwrap()
    };
    let print = || async {
        p.eval::<bool>("() => matchMedia('print').matches")
            .await
            .unwrap()
    };
    assert_eq!(screen().await, true);
    assert_eq!(print().await, false);
    p.emulate_media_builder()
        .media(Some(Media::Print))
        .emulate_media()
        .await
        .unwrap();
    assert_eq!(screen().await, false);
    assert_eq!(print().await, true);
    p.emulate_media_builder().emulate_media().await.unwrap();
    assert_eq!(screen().await, true);
    assert_eq!(print().await, false);
}

async fn check_should_work(c: &BrowserContext) {
    let p = new(c).await;
    p.set_content_builder(r#"<input type="checkbox" />"#)
        .set_content()
        .await
        .unwrap();
    p.check_builder("input").check().await.unwrap();
    let checked = p.is_checked("input", None).await.unwrap();
    assert_eq!(checked, true);
    p.uncheck_builder("input").uncheck().await.unwrap();
    let checked = p.is_checked("input", None).await.unwrap();
    assert_eq!(checked, false);
    close(&p).await;
}

async fn title_should_work(p: &Page) {
    p.eval::<String>(r#"() => document.title = "foo""#)
        .await
        .unwrap();
    assert_eq!(p.title().await.unwrap(), "foo");
}

async fn pointer(c: &BrowserContext) {
    let p = new(c).await;
    p.set_content_builder(r#"<input type="checkbox" />"#)
        .set_content()
        .await
        .unwrap();
    let checked = || async {
        p.eval::<bool>("() => document.querySelector('input').checked")
            .await
            .unwrap()
    };
    p.tap_builder("input").tap().await.unwrap();
    assert_eq!(checked().await, true);
    p.dblclick_builder("input").dblclick().await.unwrap();
    assert_eq!(checked().await, true);
    p.click_builder("input").click().await.unwrap();
    assert_eq!(checked().await, false);
    close(&p).await;
}

async fn new(c: &BrowserContext) -> Page { c.new_page().await.unwrap() }

async fn close(p: &Page) { p.close(None).await.unwrap() }

async fn text(p: &Page) {
    // TODO
}

async fn set_extra_http_headers(c: &BrowserContext, port: u16) {
    let p = new(c).await;
    p.set_extra_http_headers(vec![("hoge".into(), "hoge".into())])
        .await
        .unwrap();
    let url = super::url_static(port, "/empty.html");
    let (maybe_request, _) = tokio::join!(
        p.expect_event(page::EventType::Request),
        p.goto_builder(&url).goto()
    );
    let req = match maybe_request.unwrap() {
        page::Event::Request(req) => req,
        _ => unreachable!()
    };
    let headers = req.headers().unwrap();
    assert_eq!(headers.get("foo").unwrap(), "bar"); // set by BrowserContext
    assert_eq!(headers.get("hoge").unwrap(), "hoge");
    close(&p).await;
}

async fn query_selector_and_eval(p: &Page) {
    p.set_content_builder("").set_content().await.unwrap();
    let (wait, _) = tokio::join!(
        p.wait_for_selector_builder("div").wait_for_selector(),
        p.set_content_builder("<article><h1>foo</h1><div><div></div></div></article>")
            .set_content()
    );
    let found = wait.unwrap().unwrap();
    let divs = p.query_selector_all("div").await.unwrap();
    assert_eq!(divs.len(), 2);
    assert_eq!(&found, &divs[0]);
}
