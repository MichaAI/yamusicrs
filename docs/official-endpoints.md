# Эндпоинты официальных клиентов

Снято 2026-11-04: десктоп 5.63.1 (метод, глагол, путь) и Android 2026.09.5 (retrofit-аннотации). Новые запросы — только из этого списка (антифрод).

## Десктоп 5.63.1

```
about get account/about
acceptInvite post account/family/accept-invite/{t.inviteId}
action post dynamic-pages/trigger/action
changePlaylistDescription post users/{t.userId /playlists/}{t.playlistKind /description}
changePlaylistRelative post users/{t.userId /playlists/}{t.playlistKind /change-relative}
changePlaylistTitle post users/{t.userId /playlists/}{t.playlistKind /name}
changeTrack post ugc/tracks/{t.trackId /change}
clearSearchHistory get users/{t.userId /search-history/clear}
combinedLanding post rotor/combined/session/landing
combinedSessionNew post rotor/combined/session/new
combinedSessionNext post rotor/combined/session/{t.sessionid /next}
createPlaylist post users/{t.userId /playlists/create}
deletePlaylist post users/{t.userId /playlists/}{t.playlistKind /delete}
dislikeArtist post users/{t.userId /dislikes/artists/add}
dislikeTrack post users/{t.userId /dislikes/tracks/add}
experimentsDetails get account/experiments/details
experiments get account/experiments
getAboutArtist get artists/{t.artistId /about-artist}
getAfterTrack get after-track
getAlbumCpaId get promo/albums/{t.albumId /cpa-id}
getAlbumDonations get donation/albums/{t.albumId}
getAlbums get albums
getAlbums get labels/{t.labelId /albums}
getAlbumWithRichTracks get albums/{t.albumId /with-tracks}
getAlbumWithTracksIds get albums/{t.albumId /with-tracks}
getAllIds get library/all-ids
getAlsoAlbums get artists/{t.artistId /also-albums}
getArtists get labels/{t.labelId /artists}
getArtistSlides get rewind/slides/artist/{t.artistId}
getArtistTrackIds get artists/{t.artistId /track-ids}
getArtistTracks get artists/{t.artistId /tracks}
getAvailabilityTracks post availability/tracks
getBlockEntities get landing/block/{t.blockId /entities}
getBriefInfo get artists/{t.artistId /brief-info}
getCategoryAlbums get non-music/category/{t.id /albums}
getCategory get non-music/category/{t.categoryId}
getChart get landing3/chart
getChartPodcastsCategory get chart/podcasts/category/{t.categoryId}
getChartPodcasts get chart/podcasts
getClip get clips
getClips get artists/{t.artistId /blocks/artist-clips}
getClipsWillLike get clips/will/like
getCollectionKidsAlbumsLiked get landing-blocks/collection/kids/albums-liked
getCollectionKidsPlaylistsLiked get landing-blocks/collection/kids/playlists-liked
getCollectionKidsTracksLiked get landing-blocks/collection/kids/tracks-liked
getCompositeOffers get proxy/plus-cg-api/offers/composite
getConcerts get artists/{t.artistId /concerts}
getCreatedPlaylists get users/{t.userId /playlists/list}
getCredits get clips/{t.clipId /credits}
getCredits get tracks/{t.trackId /credits}
getData get labels/{t.labelId}
getDirectAlbums get artists/{t.artistId /direct-albums}
getDisclaimer get albums/{t.albumId /disclaimer}
getDisclaimer get artists/{t.artistId /disclaimer}
getDisclaimer get clips/{t.clipId /disclaimer}
getDisclaimer get tracks/{t.trackId /disclaimer}
getDisclaimers get disclaimers
getDiscographyAlbums get artists/{t.artistId /discography-albums}
getDislikedArtists get users/{t.userId /dislikes/artists}
getDislikedTracks get users/{t.userId /dislikes/tracks}
getEditorialAlbum get children-landing/editorial/album/{t.id}
getEditorialAlbums get non-music/editorial/album/{t.id}
getEditorialPlaylist get children-landing/editorial/playlist/{t.id}
getFamiliarYou get artists/{t.artistId /familiar-you}
getFamiliarYouInfo get artists/{t.artistId /familiar-you/info}
getFeed get concerts/feed
getFileInfoBatch get get-file-info/batch
getFileInfo get get-file-info
getFilterName get filters/{t.filterId}
getFullDescriptionTrack get tracks/{t.trackId /supplement}
getFullInfoTrack get tracks/{s /full-info}
getGenerativeInfo get rotor/station/{t.stationId /stream}
getInfo get artists/{t.artistId /info}
getInstantMixedSearch get search/instant/mixed
getInviteInfo get account/family/invite-info/{t.inviteId /}
getLikedAlbums get users/{t.userId /likes/albums/page}
getLikedArtists get users/{t.userId /likes/artists/page}
getLikedClips get users/{t.userId /likes/clips}
getLikedPlaylists get users/{t.userId /likes/playlists}
getLyrics get tracks/{t.trackId /lyrics}
getMetatagAlbums get metatags/{t.id /albums}
getMetatagArtists get metatags/{t.id /artists}
getMetatagById get metatags/{t.id}
getMetatagPlaylists get metatags/{t.id /playlists}
getMetatags get landing3/metatags
getMusicHistory get music-history
getMusicHistoryItems post music-history/items
getNewEpisodes get non-music/new-episodes
getPins get pins
getPlaylist get playlist/{t.playlistUuid}
getPlaylistIds get tags/{t.id /playlist-ids}
getPlaylistPersonal get playlists/personal/{t.playlistId}
getPlaylistsByKinds post users/{t.userId /playlists}
getPlaylists get non-music/editorial/playlist/{t.categoryId /}
getPlaylists get playlists
getPlaylistsKinds get users/{t.userId /playlists/list/kinds}
getPlaylistTrailer get users/{t.userId /playlists/}{t.playlistKind /trailer}
getPlaylistWithRichTracks get users/{t.userId /playlists/}{t.playlistKind}
getPlaylistWithTracksIds get users/{t.userId /playlists/}{t.playlistKind}
getPodcastSlides get rewind/slides/podcast/{t.podcastId}
getPresaves get users/{t.userId /presaves}
getPromoAlbumInfo get promo/albums/{t.albumId}
getPromotionsById get feed/promotions/{t.promoId}
getRedAlerts get proxy/plus-red-alert/v1/alerts
getRelatedContent get albums/{t.albumId /related-content}
getSafeDirectAlbums get artists/{t.artistId /safe-direct-albums}
getSearchHistory get users/{t.userId /search-history}
getShelfLiked get non-music/bookshelf/liked
getShelfRecentlyPlayed get non-music/bookshelf/recently-played
getSimilarArtists get artists/{t.artistId /similar}
getSkeleton get artists/{t.artistId /skeletons/}{t.skeletonId}
getSkeleton get children-landing/category/{t.categoryId}
getSkeleton get landing/skeleton/{t.id}
getStationInfo get rotor/station/{t.stationId /info}
getTabConfig get concerts/tab-config
getTopArtists get personal/top/artists/month
getTopByGenre get top/{t.category}
getTrackIds get search-playlist/{t.uid /}{t.kind}
getTracksMeta post tracks
getTrailer get albums/{t.albumId /trailer}
getTrailer get artists/{t.artistId /trailer}
getTrailer get tracks/{t.trackId /trailer}
getTrigger get dynamic-pages/trigger/polling/{t.anchorId}
getTriggers post dynamic-pages/trigger/polling
getUploadUrl post loader/upload-url
getUserSlides get rewind/slides/user
getWizardGenres get feed/wizard2-new/get-genres
infiniteFeed get infinite-feed
likeAlbum post users/{t.userId /likes/albums/add}
likeArtist post users/{t.userId /likes/artists/add}
likeClip post users/{t.userId /likes/clips/add}
likePlaylist post users/{t.userId /likes/playlists/add}
likeTrack post users/{t.userId /likes/tracks/add}
loadSession post blackbox
log post log
markAlbumFinished post streams/progress/mark-album-finished
markAlbumUnfinished post streams/progress/mark-album-unfinished
markFinished post streams/progress/mark-finished
markUnfinished post streams/progress/mark-unfinished
presaveAlbum post users/{t.userId /presaves/add}
progressSaveCurrent post streams/progress/save-current
progressSync post streams/progress/sync
removePresaveAlbum post users/{t.userId /presaves/remove}
saveAds post ads/save-ads
sendPlays post plays
settings post account/settings
shown post dynamic-pages/trigger/shown
stationFeedback post rotor/station/{t.stationId /feedback}
stationsDashboard get rotor/stations/dashboard
sync post collection/sync
togglePlaylistVisibility post users/{t.userId /playlists/}{t.playlistKind /visibility}
undislikeArtist post users/{t.userId /dislikes/artists/}{t.entityId /remove}
undislikeTrack post users/{t.userId /dislikes/tracks/}{t.entityId /remove}
unlikeAlbum post users/{t.userId /likes/albums/}{t.entityId /remove}
unlikeArtist post users/{t.userId /likes/artists/}{t.entityId /remove}
unlikeClip post users/{t.userId /likes/clips/}{t.entityId /remove}
unlikePlaylist post users/{t.userId /likes/playlists/}{t.ownerId -}{t.kindId /remove}
unlikeTrack post users/{t.userId /likes/tracks/}{t.entityId /remove}
uploadPlaylistCover post users/{t.userId /playlists/}{t.playlistKind /cover/upload}
waveLast get rotor/wave/last
waveLastReset post rotor/wave/last/reset
waveSettings get rotor/wave/settings
```

## Android 2026.09.5

```
GET account/about
GET account/experiments/details
GET account/settings
GET albums
GET albums/{albumId}
GET albums/{albumId}/disclaimer
GET albums/{albumId}/related-content
GET albums/{albumId}/similar-entities
GET albums/{albumId}/trailer
GET albums/{albumId}/with-tracks
GET api/v14/mobile/subscriptions/status
GET api/v14/users/status
GET /artists/{artistId}/about-artist
GET artists/{artistId}/also-albums?sort-by=year
GET artists/{artistId}/concerts
GET artists/{artistId}/direct-albums?sort-by=year&discographyBlockEnabled=true
GET artists/{artistId}/disclaimer
GET artists/{artistId}/discography-albums?sort-by=year
GET artists/{artistId}/familiar-you?waveTracksLimit=100
GET /artists/{artistId}/info
GET artists/{artistId}/safe-direct-albums?limit=32
GET artists/{artistId}/skeletons/{skeletonId}
GET artists/{artistId}/tracks
GET artists/{artistId}/trailer
GET artists/{id}/brief-info
GET chart/albums
GET chart/podcasts
GET chart/podcasts/category/{categoryId}
GET children-landing/category/{name}/albums
GET children-landing/compilations/{compilationId}
GET children-landing/editorial/album/{name}
GET children-landing/editorial/playlist/{name}
GET clips/{videoClipId}
GET clips/{videoclip-id}/credits
GET /concerts/{concertId}/info
GET concerts/{concertId}/skeletons/{skeletonId}
GET concerts/details/{type}/{id}
GET concerts/feed
GET concerts/locations
GET concerts/tab-config
GET custom-wave/animations
GET /custom-wave/wave-button/personal-collection
GET disclaimers
GET donation/albums/{albumId}
GET dynamic-pages/div-screens/{screenId}
GET /dynamic-pages/splash
GET feed/playlists/seen
GET feed/promotions/{id}
GET feed/wizard/entry-block
GET filters/{id}
GET get-file-info
GET global/retry-policy
GET landing3/chart
GET landing/block/{blockId}/entities
GET landing/block/mixes?fullList=true
GET landing-blocks/entities/tag/{metaTagId}/block/{metaTagType}
GET /landing-blocks/non-music/continue-listen?type=ALL
GET landing/skeleton/{id}
GET landing/special-header
GET /lumen
GET metatags/{id}/albums
GET metatags/{id}/artists
GET metatags/{id}/playlists
GET music-history
GET non-music/blocks/books/{albumId}/about
GET non-music/blocks/podcasts/{albumId}/about
GET non-music/blocks/podcasts/episodes/{trackId}/about
GET non-music/bookshelf
GET non-music/bookshelf/recently-played
GET non-music/catalogue
GET non-music/catalogue?requestedBlocks=CONTINUE_LISTEN
GET non-music/category/{name}/albums
GET non-music/compilations/{compilationId}
GET non-music/editorial/album/{name}
GET non-music/editorial/playlist/{name}
GET non-music/new-episodes
GET /personal/top/artists/month
GET playlists/personal/{playlistName}
GET playlists/{playlistId}/similar-entities
GET playlist/{uuid}
GET playlist/{uuid}?rich-tracks=true
GET prefetch.txt
GET radio/v1/regions/landing
GET radio/v1/regions/{regionId}
GET radio/v1/user/region
GET /recap-slides/artist/{artistId}
GET /recap-slides/kids
GET /recap-slides/podcast/{podcastId}
GET /recap-slides/special/{campaignId}
GET /recap-slides/user
GET resources/v2/file-info
GET rewind/cards/promo/{id}
GET rotor/session/{sessionId}?includeWaveModel=true
GET rotor/station/{stationId}/info
GET rotor/station/{stationId}/stream
GET rotor/wave/last
GET rotor/wave/settings
GET search/instant/mixed?withLikesCount=true
GET tags/{id}/playlist-ids
GET tracks/{id}/supplement
GET tracks/{track-id}/credits
GET tracks/{trackId}/disclaimer
GET tracks/{trackId}/download-info
GET tracks/{trackId}/lyrics
GET tracks/{trackId}/similar
GET tracks/{trackId}/similar-entities
GET tracks/{trackId}/trailer
GET users/{owner}/playlists/special/{type}
GET users/{owner-uid}/playlists/{kind}
GET users/{owner-uid}/playlists/{kind}?rich-tracks=true
GET users/{owner-uid}/playlists?rich-tracks=false&withLikesCount=true
GET users/{owner-uid}/search-history/clear
GET users/{userId}/playlists/{playlistKind}/trailer
GET /users/{userId}/presaves
GET /v1/sdk/configuration
GET /wave/rooms
GET /wave/rooms/{roomId}
GET wizard3/finish
GET wizard3/is-need-to-pass
POST account/feedback
POST account/settings
POST api/v14/mobile/subscriptions/google-play/submit-receipt
POST api/v1/widget/checkout/screen/closing-offer
POST api/v1/widget/checkout/screen/counter-offer
POST api/v1/widget/checkout/screen/presale
POST api/v1/widget/configurator/screen
POST /api/v2/offers
POST api/v2/widget/checkout/screen/pay-card-screen/availability
POST artists
POST /artists/blocks/player-info
POST availability/tracks/sync
POST clips
POST collection/sync
POST /concerts/landing/artists
POST dynamic-pages/mediacrm/event
POST dynamic-pages/promocodes/consume
POST dynamic-pages/promocodes/status
POST dynamic-pages/v2/trigger/action
POST dynamic-pages/v2/trigger/polling
POST dynamic-pages/v2/trigger/shown
POST dynamic-pages/v3/trigger/action
POST dynamic-pages/v3/trigger/polling
POST dynamic-pages/v3/trigger/shown
POST /events/send
POST filters/tracks
POST lyric-views
POST music-history/items
POST player-informers
POST playlists/list
POST plays
POST promo/feedback
POST push/update-token
POST render
POST rotor/combined/session/new
POST rotor/combined/session/{radioSessionId}/next
POST rotor/get-cache-recommendation
POST rotor/get-offline-recommender
POST rotor/session/new
POST rotor/session/{radioSessionId}/feedback
POST rotor/session/{radioSessionId}/tracks
POST rotor/session/{sessionId}/clone
POST rotor/sessions/feedbacks
POST rotor/station/{stationId}/feedback
POST rotor/wave/last/reset
POST search/feedback
POST search/suggestions/q2v/feedback
POST streams/progress/sync
POST tracks?with-positions=true
POST users/{currentUserId}/dislikes/tracks/actions/add
POST users/{currentUserId}/likes/tracks/actions/add
POST users/{currentUserId}/likes/tracks/actions/remove
POST users/{id}/dislikes/artists/add-multiple
POST users/{id}/likes/albums/add-multiple
POST users/{id}/likes/albums/remove
POST users/{id}/likes/artists/add-multiple
POST users/{id}/likes/artists/remove
POST users/{id}/likes/playlists/add-multiple
POST users/{id}/likes/playlists/remove
POST users/{owner-uid}/playlists/create
POST users/{owner-uid}/playlists/{kinds}/change-relative
POST users/{owner-uid}/playlists/{kinds}/cover/clear
POST users/{owner-uid}/playlists/{kinds}/cover/upload
POST users/{owner-uid}/playlists/{kinds}/delete
POST users/{owner-uid}/playlists/{kinds}/description
POST users/{owner-uid}/playlists/{kinds}/name
POST users/{owner-uid}/playlists/{kinds}/visibility
POST /users/{userId}/dislikes/clips/actions/add
POST /users/{userId}/likes/clips/actions/add
POST /users/{userId}/likes/clips/actions/remove
POST /users/{userId}/presaves/add
POST /users/{userId}/presaves/remove
POST /v1/missions/mission
POST /v1/missions/mission/activate
POST /v1/missions/missions
POST /v1/plus-state
POST /v1/pullnotifier/plaque/clicked
POST /v1/pullnotifier/plaques
POST /v1/pullnotifier/plaque/seen
POST /wave/rooms
POST welcome-screen/gdpr/feedback
POST wheel/feedbacks
POST wheel/new
POST wizard3/initial-artists
POST wizard3/like
POST wizard3/similar-artists
POST wizard3/similar-artists-by-tap
POST wizard3/unlike
POST /words/big-cards
POST /words/cards
```
