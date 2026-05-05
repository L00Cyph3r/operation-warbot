let eventSource = new EventSource('/sse/tiltify');

eventSource.onmessage = function (event) {
    try {
        let json = JSON.parse(event.data);
        if (json.event_type === "DonationUpdated") {

        } else {
            if (json.data['total_amount_raised']) {
                let total_amount_raised = json.data['total_amount_raised'];
                document.getElementById('total').innerText = '$' + total_amount_raised.value;
            }
        }

        // {"event_type":"DonationUpdated","datetime":"2025-05-13T01:36:11.351703Z","amount":{"currency":"USD","value":"4.20"},"name":"L00_Cyph3r","message":"Testing in production is fun!"}
    } catch (e) {
        return;
    }

    console.log('Message from server ', event.data);
}

fetch('/tiltify_team_stats.json').then((res) => {
    res.json().then((json) => {
        let total_amount_raised = json.data.total_amount_raised;
        document.getElementById('total').innerText = '$' + total_amount_raised.value;
    })
});